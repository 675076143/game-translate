mod capture;
mod confirm;
mod dedup;
mod logger;
mod ocr;
mod output;
mod stability;
mod terminology;
mod translate;
mod window;

use std::{
    collections::HashMap,
    env,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use capture::{Capture, Geometry};
use confirm::CandidateConfirmer;
use dedup::TextDeduplicator;
use ocr::OcrResult;
use output::{print_error, print_status, print_translation};
use stability::{FrameEvent, StabilityDetector};
use translate::{TranslationJob, TranslationOutcome, Translator};

const LAYOUT_SETTLE: Duration = Duration::from_millis(1_500);
const ACTIVE_INTERVAL: Duration = Duration::from_millis(80);
const IDLE_INTERVAL: Duration = Duration::from_millis(200);
const WINDOW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const SPECULATIVE_CONFIDENCE: f32 = 60.0;

fn main() {
    if let Err(error) = run() {
        print_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let log_path = logger::init()?;
    let cache_path = log_path
        .parent()
        .context("日志路径没有父目录")?
        .join("translations.jsonl");
    let (translation_tx, translation_rx) = translation_worker(cache_path);
    print_status("等待平铺布局稳定…");
    thread::sleep(LAYOUT_SETTLE);
    let (tracker, detect_panel) = match env::args().nth(1).as_deref() {
        None | Some("--region") => {
            let selection = select_region()?;
            (window::WindowTracker::bind(selection)?, false)
        }
        Some("--window") => {
            print_status("请点击要跟踪的游戏窗口");
            (window::WindowTracker::select()?, true)
        }
        Some(_) => bail!("usage: game-translate [--region|--window]"),
    };
    let mut geometry = tracker.geometry()?.context("游戏窗口当前不可见")?;
    let mut capture = Capture::new(geometry)?;
    let mut stability = StabilityDetector::new();
    let mut confirmer = CandidateConfirmer::new();
    let mut dedup = TextDeduplicator::default();
    let mut next_window_refresh = Instant::now() + WINDOW_REFRESH_INTERVAL;
    let mut window_visible = true;
    let mut change_started = None;
    let mut next_event_id = 1_u64;
    let mut latest_confirmed = None;
    let mut speculative: Option<(String, u64)> = None;
    let mut buffered = HashMap::new();
    let mut event_started = HashMap::new();

    print_status(&format!(
        "已绑定「{}」的字幕区域 {}×{}；日志：{}",
        tracker.title(),
        geometry.width,
        geometry.height,
        log_path.display()
    ));

    loop {
        while let Ok(outcome) = translation_rx.try_recv() {
            process_outcome(
                outcome,
                latest_confirmed,
                speculative.as_ref().map(|item| item.1),
                &mut buffered,
                &mut event_started,
            );
        }

        if Instant::now() >= next_window_refresh {
            next_window_refresh = Instant::now() + WINDOW_REFRESH_INTERVAL;
            match tracker.geometry()? {
                Some(updated) => {
                    if !window_visible {
                        print_status("游戏窗口已恢复，继续监视");
                        window_visible = true;
                    }
                    if updated != geometry {
                        capture.set_geometry(updated);
                        geometry = updated;
                        stability = StabilityDetector::new();
                        confirmer.cancel();
                        speculative = None;
                        logger::write(
                            "window",
                            &format!(
                                "region moved to {},{} {}x{}",
                                updated.x, updated.y, updated.width, updated.height
                            ),
                        );
                    }
                }
                None => {
                    if window_visible {
                        print_status("游戏窗口不可见，已暂停识别");
                        window_visible = false;
                        confirmer.cancel();
                        speculative = None;
                    }
                    wait_for_translation(
                        &translation_rx,
                        IDLE_INTERVAL,
                        latest_confirmed,
                        None,
                        &mut buffered,
                        &mut event_started,
                    );
                    continue;
                }
            }
        }

        let iteration_started = Instant::now();
        let capture_started = Instant::now();
        let frame = capture.frame().context("Wayland 截图失败")?;
        let capture_elapsed = capture_started.elapsed();
        let focused = ocr::focus(&frame, detect_panel);
        let event = stability.update(&focused);

        if event == FrameEvent::Changed {
            change_started = Some(Instant::now());
            logger::write("change", "detected");
            confirmer.cancel();
            speculative = None;
        } else if event == FrameEvent::Stable {
            let ocr_started = Instant::now();
            let recognition = if detect_panel {
                ocr::recognize_window(&frame)
            } else {
                ocr::recognize(&focused, false)
            };
            match recognition {
                Ok(result) => {
                    logger::write(
                        "ocr-candidate",
                        &format!(
                            "capture_ms={} confidence={:.1} elapsed_ms={} since_change_ms={} text={}",
                            capture_elapsed.as_millis(),
                            result.confidence,
                            ocr_started.elapsed().as_millis(),
                            change_started.map_or(0, |start| start.elapsed().as_millis()),
                            result.text
                        ),
                    );
                    confirmer.propose(result.clone(), Instant::now());
                    if is_speculative_candidate(&result) {
                        let id = next_event_id;
                        next_event_id += 1;
                        translation_tx
                            .send(TranslationJob {
                                id,
                                original: result.text.clone(),
                            })
                            .context("翻译线程已退出")?;
                        speculative = Some((result.text, id));
                        event_started.insert(id, change_started.unwrap_or_else(Instant::now));
                        logger::write("translate-speculative", &format!("id={id}"));
                    }
                }
                Err(error) => logger::write(
                    "ocr-rejected",
                    &format!(
                        "capture_ms={} elapsed_ms={} error={error:#}",
                        capture_elapsed.as_millis(),
                        ocr_started.elapsed().as_millis()
                    ),
                ),
            }
        } else if confirmer.is_due(Instant::now()) {
            confirm_candidate(
                &frame,
                &mut confirmer,
                &mut dedup,
                &translation_tx,
                &mut next_event_id,
                &mut latest_confirmed,
                &mut speculative,
                &mut buffered,
                &mut event_started,
                &mut change_started,
                detect_panel,
            )?;
        }

        let base_interval = if stability.is_active() {
            ACTIVE_INTERVAL
        } else {
            IDLE_INTERVAL
        };
        let mut wait = base_interval.saturating_sub(iteration_started.elapsed());
        wait = wait.min(next_window_refresh.saturating_duration_since(Instant::now()));
        if let Some(remaining) = confirmer.remaining(Instant::now()) {
            wait = wait.min(remaining);
        }
        wait_for_translation(
            &translation_rx,
            wait,
            latest_confirmed,
            speculative.as_ref().map(|item| item.1),
            &mut buffered,
            &mut event_started,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn confirm_candidate(
    frame: &image::DynamicImage,
    confirmer: &mut CandidateConfirmer,
    dedup: &mut TextDeduplicator,
    translation_tx: &mpsc::Sender<TranslationJob>,
    next_event_id: &mut u64,
    latest_confirmed: &mut Option<u64>,
    speculative: &mut Option<(String, u64)>,
    buffered: &mut HashMap<u64, TranslationOutcome>,
    event_started: &mut HashMap<u64, Instant>,
    change_started: &mut Option<Instant>,
    window_mode: bool,
) -> Result<()> {
    let ocr_started = Instant::now();
    let recognition = if window_mode {
        ocr::recognize_window(frame)
    } else {
        ocr::recognize(frame, false)
    };
    match recognition {
        Ok(result) => {
            logger::write(
                "ocr-confirm",
                &format!(
                    "confidence={:.1} elapsed_ms={} text={}",
                    result.confidence,
                    ocr_started.elapsed().as_millis(),
                    result.text
                ),
            );
            if let Some(confirmed) = confirmer.confirm(result, Instant::now())
                && dedup.is_new(&confirmed.text)
            {
                let id = if let Some(id) = speculative
                    .as_ref()
                    .filter(|item| item.0 == confirmed.text)
                    .map(|item| item.1)
                {
                    id
                } else {
                    let id = *next_event_id;
                    *next_event_id += 1;
                    translation_tx
                        .send(TranslationJob {
                            id,
                            original: confirmed.text.clone(),
                        })
                        .context("翻译线程已退出")?;
                    id
                };
                *latest_confirmed = Some(id);
                event_started
                    .entry(id)
                    .or_insert_with(|| change_started.unwrap_or_else(Instant::now));
                logger::write(
                    "confirmed",
                    &format!(
                        "id={id} since_change_ms={}",
                        change_started.map_or(0, |start| start.elapsed().as_millis())
                    ),
                );
                *change_started = None;
                if let Some(outcome) = buffered.remove(&id) {
                    render_outcome(outcome, event_started);
                }
            } else if let Some((_, id)) = speculative.as_ref() {
                buffered.remove(id);
                event_started.remove(id);
                logger::write("duplicate", &format!("id={id}"));
            }
            *speculative = None;
        }
        Err(error) => {
            confirmer.cancel();
            *speculative = None;
            logger::write(
                "ocr-rejected",
                &format!(
                    "elapsed_ms={} error={error:#}",
                    ocr_started.elapsed().as_millis()
                ),
            );
        }
    }
    Ok(())
}

fn wait_for_translation(
    receiver: &mpsc::Receiver<TranslationOutcome>,
    duration: Duration,
    latest_confirmed: Option<u64>,
    speculative_id: Option<u64>,
    buffered: &mut HashMap<u64, TranslationOutcome>,
    event_started: &mut HashMap<u64, Instant>,
) {
    if let Ok(outcome) = receiver.recv_timeout(duration) {
        process_outcome(
            outcome,
            latest_confirmed,
            speculative_id,
            buffered,
            event_started,
        );
    }
}

fn process_outcome(
    outcome: TranslationOutcome,
    latest_confirmed: Option<u64>,
    speculative_id: Option<u64>,
    buffered: &mut HashMap<u64, TranslationOutcome>,
    event_started: &mut HashMap<u64, Instant>,
) {
    if latest_confirmed == Some(outcome.id) {
        render_outcome(outcome, event_started);
    } else if speculative_id == Some(outcome.id) {
        buffered.insert(outcome.id, outcome);
    } else {
        logger::write("translate-stale", &format!("id={}", outcome.id));
        event_started.remove(&outcome.id);
    }
}

fn render_outcome(outcome: TranslationOutcome, event_started: &mut HashMap<u64, Instant>) {
    let id = outcome.id;
    match outcome.result {
        Ok(translation) => {
            let total = event_started
                .remove(&translation.id)
                .map_or(0, |started| started.elapsed().as_millis());
            logger::write(
                "translate-complete",
                &format!(
                    "id={} source={} translate_ms={} total_ms={total}",
                    translation.id,
                    translation.source.as_str(),
                    translation.elapsed.as_millis()
                ),
            );
            print_translation(&translation.original, &translation.translated);
        }
        Err(error) => {
            let total = event_started
                .remove(&id)
                .map_or(0, |started| started.elapsed().as_millis());
            logger::write(
                "translate-complete",
                &format!("id={id} success=false total_ms={total}"),
            );
            print_error(&error.to_string());
        }
    }
}

fn is_speculative_candidate(result: &OcrResult) -> bool {
    result.confidence >= SPECULATIVE_CONFIDENCE && result.text.ends_with(['.', '!', '?'])
}

fn select_region() -> Result<Geometry> {
    print_status("请框选字幕文字区域（不要包含对话框边框）");
    let output = Command::new("slurp")
        .arg("-d")
        .output()
        .context("无法启动 slurp")?;
    if !output.status.success() {
        bail!("已取消字幕区域选择");
    }
    let selected = String::from_utf8(output.stdout).context("slurp 返回了无效文本")?;
    selected.trim().parse()
}

fn translation_worker(
    cache_path: std::path::PathBuf,
) -> (
    mpsc::Sender<TranslationJob>,
    mpsc::Receiver<TranslationOutcome>,
) {
    let (job_tx, job_rx) = mpsc::channel::<TranslationJob>();
    let (result_tx, result_rx) = mpsc::channel::<TranslationOutcome>();
    thread::spawn(move || {
        let mut translator = match Translator::new(cache_path) {
            Ok(translator) => translator,
            Err(error) => {
                logger::write("translate-error", &error.to_string());
                return;
            }
        };
        translator.warm_up();
        while let Ok(mut job) = job_rx.recv() {
            while let Ok(newer) = job_rx.try_recv() {
                logger::write("translate-coalesced", &format!("dropped_id={}", job.id));
                job = newer;
            }
            let id = job.id;
            let result = translator.translate(job);
            if result_tx.send(TranslationOutcome { id, result }).is_err() {
                return;
            }
        }
    });
    (job_tx, result_rx)
}

#[cfg(test)]
mod tests {
    use super::is_speculative_candidate;
    use crate::ocr::OcrResult;

    #[test]
    fn speculates_only_on_confident_complete_sentences() {
        let result = |text: &str, confidence| OcrResult {
            text: text.into(),
            confidence,
        };
        assert!(is_speculative_candidate(&result("Hello!", 60.0)));
        assert!(!is_speculative_candidate(&result("Hello", 90.0)));
        assert!(!is_speculative_candidate(&result("Hello!", 59.9)));
    }
}
