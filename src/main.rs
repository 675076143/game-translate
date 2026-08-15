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
use output::{print_error, print_status, print_translation};
use stability::{FrameEvent, StabilityDetector};
use translate::{Translation, Translator};

const LAYOUT_SETTLE: Duration = Duration::from_millis(1_500);
const ACTIVE_INTERVAL: Duration = Duration::from_millis(120);
const IDLE_INTERVAL: Duration = Duration::from_millis(450);
const WINDOW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

fn main() {
    if let Err(error) = run() {
        print_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let log_path = logger::init()?;
    print_status("等待平铺布局稳定…");
    thread::sleep(LAYOUT_SETTLE);
    let tracker = match env::args().nth(1).as_deref() {
        None | Some("--region") => {
            let selection = select_region()?;
            window::WindowTracker::bind(selection)?
        }
        Some("--window") => {
            print_status("请点击要跟踪的游戏窗口");
            window::WindowTracker::select()?
        }
        Some(_) => bail!("usage: game-translate [--region|--window]"),
    };
    let mut geometry = tracker.geometry()?.context("游戏窗口当前不可见")?;
    let mut capture = Capture::new(geometry)?;
    let mut stability = StabilityDetector::new();
    let mut confirmer = CandidateConfirmer::new();
    let mut dedup = TextDeduplicator::default();
    let (translation_tx, translation_rx) = translation_worker();
    let mut next_window_refresh = Instant::now() + WINDOW_REFRESH_INTERVAL;
    let mut window_visible = true;

    print_status(&format!(
        "已绑定「{}」的字幕区域 {}×{}；日志：{}",
        tracker.title(),
        geometry.width,
        geometry.height,
        log_path.display()
    ));

    loop {
        while let Ok(result) = translation_rx.try_recv() {
            match result {
                Ok(translation) => {
                    print_translation(&translation.original, &translation.translated)
                }
                Err(error) => print_error(&error.to_string()),
            }
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
                    }
                    thread::sleep(IDLE_INTERVAL);
                    continue;
                }
            }
        }

        let started = Instant::now();
        let frame = capture.frame().context("Wayland 截图失败")?;
        let event = stability.update(&frame);

        if event == FrameEvent::Changed {
            confirmer.cancel();
        } else if event == FrameEvent::Stable {
            let ocr_started = Instant::now();
            match ocr::recognize(&frame) {
                Ok(result) => {
                    logger::write(
                        "ocr-candidate",
                        &format!(
                            "confidence={:.1} elapsed_ms={} text={}",
                            result.confidence,
                            ocr_started.elapsed().as_millis(),
                            result.text
                        ),
                    );
                    confirmer.propose(result, Instant::now());
                }
                Err(error) => logger::write(
                    "ocr-rejected",
                    &format!(
                        "elapsed_ms={} error={error:#}",
                        ocr_started.elapsed().as_millis()
                    ),
                ),
            }
        } else if confirmer.is_due(Instant::now()) {
            let ocr_started = Instant::now();
            match ocr::recognize(&frame) {
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
                        translation_tx
                            .send(confirmed.text)
                            .context("翻译线程已退出")?;
                    }
                }
                Err(error) => {
                    confirmer.cancel();
                    logger::write(
                        "ocr-rejected",
                        &format!(
                            "elapsed_ms={} error={error:#}",
                            ocr_started.elapsed().as_millis()
                        ),
                    );
                }
            }
        }

        let interval = if stability.is_active() {
            ACTIVE_INTERVAL
        } else {
            IDLE_INTERVAL
        };
        thread::sleep(interval.saturating_sub(started.elapsed()));
    }
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

fn translation_worker() -> (mpsc::Sender<String>, mpsc::Receiver<Result<Translation>>) {
    let (job_tx, job_rx) = mpsc::channel::<String>();
    let (result_tx, result_rx) = mpsc::channel::<Result<Translation>>();
    thread::spawn(move || {
        let translator = match Translator::new() {
            Ok(translator) => translator,
            Err(error) => {
                let _ = result_tx.send(Err(error));
                return;
            }
        };
        for original in job_rx {
            let result = translator
                .translate(&original)
                .map(|translated| Translation {
                    original,
                    translated,
                });
            if result_tx.send(result).is_err() {
                return;
            }
        }
    });
    (job_tx, result_rx)
}
