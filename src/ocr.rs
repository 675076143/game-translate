use std::{
    borrow::Cow,
    env,
    io::{BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, GenericImage, ImageFormat};
use serde_json::{Value, json};

const TOP_HEIGHT_PERCENT: u32 = 30;
const BOTTOM_Y_PERCENT: u32 = 65;
const BOTTOM_HEIGHT_PERCENT: u32 = 33;

#[derive(Debug, Clone)]
pub struct OcrResult {
    pub text: String,
    pub confidence: f32,
}

pub struct OcrEngine {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl OcrEngine {
    pub fn new() -> Result<Self> {
        let home = env::var_os("HOME").context("HOME is not set")?;
        let root = PathBuf::from(home).join(".local/share/game-translate/ocr");
        let python = env::var_os("GAME_TRANSLATE_OCR_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(".venv/bin/python"));
        let worker = env::var_os("GAME_TRANSLATE_OCR_WORKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("ocr_worker.py"));
        let mut child = Command::new(&python)
            .arg(&worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| {
                format!(
                    "无法启动 RapidOCR；请先安装 OCR 运行时：{} {}",
                    python.display(),
                    worker.display()
                )
            })?;
        let stdin = child.stdin.take().context("RapidOCR stdin unavailable")?;
        let stdout = BufReader::new(child.stdout.take().context("RapidOCR stdout unavailable")?);
        Ok(Self {
            _child: child,
            stdin,
            stdout,
        })
    }

    pub fn recognize(&mut self, image: &DynamicImage) -> Result<OcrResult> {
        let mut png = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut png, ImageFormat::Png)
            .context("无法编码 RapidOCR 输入")?;
        let request = serde_json::to_vec(&json!({ "png": STANDARD.encode(png.into_inner()) }))?;
        let size = u32::try_from(request.len()).context("RapidOCR request is too large")?;
        self.stdin.write_all(&size.to_le_bytes())?;
        self.stdin.write_all(&request)?;
        self.stdin.flush()?;

        let mut header = [0_u8; 4];
        self.stdout
            .read_exact(&mut header)
            .context("RapidOCR worker exited")?;
        let mut response = vec![0; u32::from_le_bytes(header) as usize];
        self.stdout.read_exact(&mut response)?;
        parse_response(serde_json::from_slice(&response)?)
    }
}

pub fn focus(image: &DynamicImage, detect_panel: bool) -> Cow<'_, DynamicImage> {
    if !detect_panel {
        return Cow::Borrowed(image);
    }
    let top_height = image.height() * TOP_HEIGHT_PERCENT / 100;
    let bottom_y = image.height() * BOTTOM_Y_PERCENT / 100;
    let bottom_height = image.height() * BOTTOM_HEIGHT_PERCENT / 100;
    let mut hud = DynamicImage::new_rgba8(image.width(), top_height + bottom_height);
    hud.copy_from(&image.crop_imm(0, 0, image.width(), top_height), 0, 0)
        .expect("HUD top band dimensions must match");
    hud.copy_from(
        &image.crop_imm(0, bottom_y, image.width(), bottom_height),
        0,
        top_height,
    )
    .expect("HUD bottom band dimensions must match");
    Cow::Owned(hud)
}

fn parse_response(response: Value) -> Result<OcrResult> {
    if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        bail!(
            "{}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("RapidOCR failed")
        );
    }
    let text = response
        .get("text")
        .and_then(Value::as_str)
        .context("RapidOCR response has no text")?
        .trim()
        .to_owned();
    let result = OcrResult {
        text,
        confidence: response
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or_default() as f32,
    };
    validate_dialogue(&result)?;
    Ok(result)
}

fn validate_dialogue(result: &OcrResult) -> Result<()> {
    let words: Vec<_> = result.text.split_whitespace().collect();
    let wordlike = words
        .iter()
        .filter(|word| word.chars().filter(|character| character.is_alphabetic()).count() >= 2)
        .count();
    let contains_url = words.iter().any(|word| {
        word.starts_with("http://") || word.starts_with("https://") || word.starts_with("www.")
    });
    let complete = result.text.ends_with(['.', '!', '?']);
    let uppercase = result.text.chars().filter(|character| character.is_uppercase()).count();
    let letters = result.text.chars().filter(|character| character.is_alphabetic()).count();
    let short_ui_label = !complete
        && (wordlike < 4 || (letters > 0 && uppercase * 2 > letters));
    if result.text.is_empty()
        || result.confidence < 70.0
        || wordlike < 2
        || words.len() > 80
        || contains_url
        || short_ui_label
    {
        bail!(
            "OCR result rejected (confidence={:.1} words={} wordlike={} url={})",
            result.confidence,
            words.len(),
            wordlike,
            contains_url
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};
    use serde_json::json;

    use super::{focus, parse_response};

    #[test]
    fn parses_worker_response() {
        let result = parse_response(json!({
            "ok": true,
            "text": "You put the Potion away in the Medicine Pocket.",
            "confidence": 98.5,
            "line_count": 2,
            "word_confidences": [98.0, 99.0]
        }))
        .unwrap();
        assert_eq!(result.confidence, 98.5);
    }

    #[test]
    fn rejects_low_confidence_noise() {
        assert!(
            parse_response(json!({
                "ok": true,
                "text": "random screen noise",
                "confidence": 42.0,
                "line_count": 1,
                "word_confidences": [42.0]
            }))
            .is_err()
        );
    }

    #[test]
    fn selects_top_and_bottom_hud_bands() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(400, 400, Rgb([30, 30, 30])));
        let panel = focus(&image, true);
        assert_eq!(panel.width(), 400);
        assert_eq!(panel.height(), 252);
    }
}
