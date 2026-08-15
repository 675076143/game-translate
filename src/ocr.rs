use std::{
    borrow::Cow,
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImage, ImageFormat, imageops::FilterType};

const MIN_RESULT_CONFIDENCE: f32 = 45.0;

#[derive(Debug, Clone)]
pub struct OcrResult {
    pub text: String,
    pub confidence: f32,
}

struct OcrLine {
    key: (u32, u32, u32),
    words: Vec<String>,
    confidence_total: f32,
    word_count: usize,
}

pub fn focus(image: &DynamicImage, detect_panel: bool) -> Cow<'_, DynamicImage> {
    if detect_panel {
        let top_height = image.height() * 40 / 100;
        let bottom_y = image.height() * 55 / 100;
        let bottom_height = image.height() * 43 / 100;
        let mut hud = DynamicImage::new_rgba8(image.width(), top_height + bottom_height);
        hud.copy_from(&image.crop_imm(0, 0, image.width(), top_height), 0, 0)
            .expect("HUD top band dimensions must match");
        hud.copy_from(
            &image.crop_imm(0, bottom_y, image.width(), bottom_height),
            0,
            top_height,
        )
        .expect("HUD bottom band dimensions must match");
        return Cow::Owned(hud);
    }
    Cow::Borrowed(image)
}

pub fn recognize_window(image: &DynamicImage) -> Result<OcrResult> {
    let top_height = image.height() * 40 / 100;
    let bottom_y = image.height() * 55 / 100;
    let bottom_height = image.height() * 43 / 100;
    let top = recognize(&image.crop_imm(0, 0, image.width(), top_height), true);
    let bottom = recognize(
        &image.crop_imm(0, bottom_y, image.width(), bottom_height),
        true,
    );
    match (top, bottom) {
        (Ok(top), Ok(bottom)) => Ok(if dialogue_score(&top) >= dialogue_score(&bottom) {
            top
        } else {
            bottom
        }),
        (Ok(result), Err(_)) | (Err(_), Ok(result)) => Ok(result),
        (Err(top), Err(bottom)) => bail!("top HUD: {top:#}; bottom HUD: {bottom:#}"),
    }
}

fn dialogue_score(result: &OcrResult) -> f32 {
    let words = result.text.split_whitespace().count().min(10) as f32;
    let complete = f32::from(result.text.ends_with(['.', '!', '?'])) * 10.0;
    result.confidence + words * 3.0 + complete
}

pub fn recognize(image: &DynamicImage, block_text: bool) -> Result<OcrResult> {
    let original = recognize_prepared(&image.grayscale(), block_text);
    if original
        .as_ref()
        .is_ok_and(|result| result.confidence >= 70.0)
    {
        return original;
    }

    let scaled =
        image
            .grayscale()
            .resize_exact(image.width() * 2, image.height() * 2, FilterType::Nearest);
    let enlarged = recognize_prepared(&scaled, block_text);
    match (original, enlarged) {
        (Ok(original), Ok(enlarged)) => Ok(
            if starts_lowercase(&original.text) && starts_uppercase(&enlarged.text) {
                enlarged
            } else {
                original
            },
        ),
        (Ok(result), Err(_)) | (Err(_), Ok(result)) => Ok(result),
        (Err(original), Err(enlarged)) => {
            bail!("original: {original:#}; enlarged: {enlarged:#}")
        }
    }
}

fn starts_lowercase(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(char::is_lowercase)
}

fn starts_uppercase(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(char::is_uppercase)
}

fn recognize_prepared(image: &DynamicImage, block_text: bool) -> Result<OcrResult> {
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .context("无法编码 OCR 输入")?;

    let page_segmentation = if block_text { "6" } else { "11" };
    let mut child = Command::new("tesseract")
        .args([
            "stdin",
            "stdout",
            "-l",
            "eng",
            "--psm",
            page_segmentation,
            "tsv",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 Tesseract")?;
    child
        .stdin
        .take()
        .context("无法打开 Tesseract 输入")?
        .write_all(png.get_ref())
        .context("无法写入 Tesseract 输入")?;
    let output = child.wait_with_output().context("等待 Tesseract 失败")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let raw = String::from_utf8(output.stdout).context("Tesseract 输出不是 UTF-8")?;
    parse_tsv(&raw)
}

fn clean_word(word: &str) -> Option<String> {
    let word = if let Some((_, suffix)) = word.rsplit_once('*')
        && suffix.chars().filter(|c| c.is_alphabetic()).count() >= 2
    {
        suffix
    } else {
        word
    };
    (word.chars().any(char::is_alphanumeric) && word != "|").then(|| word.to_owned())
}

fn parse_tsv(raw: &str) -> Result<OcrResult> {
    let mut lines: Vec<OcrLine> = Vec::new();
    for row in raw.lines().skip(1) {
        let columns: Vec<_> = row.splitn(12, '\t').collect();
        if columns.len() != 12 || columns[0] != "5" || columns[11].trim().is_empty() {
            continue;
        }
        let key = (
            columns[2].parse().context("OCR block number invalid")?,
            columns[3].parse().context("OCR paragraph number invalid")?,
            columns[4].parse().context("OCR line number invalid")?,
        );
        let confidence: f32 = columns[10].parse().context("OCR confidence invalid")?;
        let Some(text) = clean_word(columns[11].trim()) else {
            continue;
        };
        if let Some(line) = lines.last_mut().filter(|line| line.key == key) {
            line.words.push(text);
            line.confidence_total += confidence;
            line.word_count += 1;
        } else {
            lines.push(OcrLine {
                key,
                words: vec![text],
                confidence_total: confidence,
                word_count: 1,
            });
        }
    }

    let accepted: Vec<_> = lines
        .into_iter()
        .filter_map(|line| {
            let text = line.words.join(" ");
            let confidence = line.confidence_total / line.word_count as f32;
            let dialogue_line = line.word_count >= 2 || text.ends_with(['.', '!', '?']);
            (dialogue_line
                && text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
                && confidence >= MIN_RESULT_CONFIDENCE)
                .then_some((text, line.confidence_total, line.word_count))
        })
        .collect();
    let count: usize = accepted.iter().map(|line| line.2).sum();
    let confidence = accepted.iter().map(|line| line.1).sum::<f32>() / count.max(1) as f32;
    let text = accepted
        .into_iter()
        .map(|line| line.0)
        .collect::<Vec<_>>()
        .join(" ");
    let looks_like_dialogue =
        text.split_whitespace().count() >= 2 || text.ends_with(['.', '!', '?']);
    if text.is_empty() || !looks_like_dialogue || confidence < MIN_RESULT_CONFIDENCE {
        bail!("OCR result rejected (confidence {confidence:.1})");
    }
    Ok(OcrResult { text, confidence })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use image::{DynamicImage, Rgb, RgbImage};

    use super::{focus, parse_tsv, recognize};

    #[test]
    fn parses_lines_and_drops_isolated_noise() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t80\tR\n\
5\t1\t3\t1\t1\t1\t0\t0\t1\t1\t1\tborder\n\
5\t1\t3\t1\t1\t2\t0\t0\t1\t1\t1\tnoise\n\
5\t1\t2\t1\t1\t1\t0\t0\t1\t1\t90\tHello\n\
5\t1\t2\t1\t1\t2\t0\t0\t1\t1\t90\tworld!\n";
        let result = parse_tsv(tsv).unwrap();
        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.confidence, 90.0);
    }

    #[test]
    fn rejects_a_single_unpunctuated_word() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t90\tFout\n";
        assert!(parse_tsv(tsv).is_err());
    }

    #[test]
    fn selects_top_and_bottom_hud_bands_regardless_of_color() {
        let image = RgbImage::from_pixel(400, 400, Rgb([30, 30, 30]));
        let image = DynamicImage::ImageRgb8(image);
        let panel = focus(&image, true);
        assert_eq!(panel.width(), 400);
        assert_eq!(panel.height(), 332);
    }

    #[test]
    fn removes_panel_borders_and_icon_artifacts() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t79\t|\n\
5\t1\t1\t1\t1\t2\t0\t0\t1\t1\t80\tYou\n\
5\t1\t1\t1\t1\t3\t0\t0\t1\t1\t80\tput\n\
5\t1\t1\t1\t1\t4\t0\t0\t1\t1\t80\tthe\n\
5\t1\t1\t1\t1\t5\t0\t0\t1\t1\t86\tPecha\n\
5\t1\t1\t1\t1\t6\t0\t0\t1\t1\t79\tBerry\n\
5\t1\t1\t1\t1\t7\t0\t0\t1\t1\t75\taway\n\
5\t1\t1\t1\t2\t1\t0\t0\t1\t1\t89\tin\n\
5\t1\t1\t1\t2\t2\t0\t0\t1\t1\t89\tthe\n\
5\t1\t1\t1\t2\t3\t0\t0\t1\t1\t0\ts*Berries\n\
5\t1\t1\t1\t2\t4\t0\t0\t1\t1\t80\tPocket.\n";
        assert_eq!(
            parse_tsv(tsv).unwrap().text,
            "You put the Pecha Berry away in the Berries Pocket."
        );
    }

    #[test]
    fn recognizes_pokemon_center_dialogue() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pokemon-center.png");
        let image = image::open(fixture).unwrap();
        assert_eq!(
            recognize(&image, false).unwrap().text,
            "We restore your tired Pokémon to full health."
        );
    }

    #[test]
    fn recognizes_red_dialogue_after_grayscale_conversion() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/red-dialogue.png");
        let image = image::open(fixture).unwrap();
        assert_eq!(
            recognize(&image, true).unwrap().text,
            "We've restored your Pokémon to full hiealth."
        );
    }

    #[test]
    fn retries_low_confidence_pixel_text_at_double_scale() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/red-we-restore.png");
        let image = image::open(fixture).unwrap();
        assert_eq!(
            recognize(&image, true).unwrap().text,
            "We restore your tired Pokemon to Tull health."
        );
    }
}
