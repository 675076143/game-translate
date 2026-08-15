mod capture;
mod dedup;
mod ocr;
mod output;
mod stability;
mod translate;

use std::{
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use capture::{Capture, Geometry};
use dedup::TextDeduplicator;
use output::{print_error, print_status, print_translation};
use stability::{FrameEvent, StabilityDetector};
use translate::{Translation, Translator};

const LAYOUT_SETTLE: Duration = Duration::from_millis(1_500);
const ACTIVE_INTERVAL: Duration = Duration::from_millis(120);
const IDLE_INTERVAL: Duration = Duration::from_millis(450);

fn main() {
    if let Err(error) = run() {
        print_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    print_status("等待平铺布局稳定…");
    thread::sleep(LAYOUT_SETTLE);
    let geometry = select_region()?;
    let mut capture = Capture::new(geometry)?;
    let mut stability = StabilityDetector::new();
    let mut dedup = TextDeduplicator::default();
    let (translation_tx, translation_rx) = translation_worker();

    print_status(&format!(
        "正在监视字幕区域 {}×{}；关闭窗口即可停止",
        geometry.width, geometry.height
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

        let started = Instant::now();
        let frame = capture.frame().context("Wayland 截图失败")?;
        let event = stability.update(&frame);

        if event == FrameEvent::Stable {
            match ocr::recognize(&frame) {
                Ok(text) if dedup.is_new(&text) => {
                    translation_tx.send(text).context("翻译线程已退出")?;
                }
                Ok(_) => {}
                Err(error) => print_error(&format!("OCR 失败：{error:#}")),
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
