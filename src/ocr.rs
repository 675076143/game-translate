use std::{
    borrow::Cow,
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageFormat};

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
        let top = image.height() * 55 / 100;
        let bottom = image.height() * 98 / 100;
        return Cow::Owned(image.crop_imm(0, top, image.width(), bottom - top));
    }
    Cow::Borrowed(image)
}

pub fn recognize(image: &DynamicImage, block_text: bool) -> Result<OcrResult> {
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
            (text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
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
    fn selects_the_bottom_dialogue_band_regardless_of_color() {
        let image = RgbImage::from_pixel(400, 400, Rgb([30, 30, 30]));
        let image = DynamicImage::ImageRgb8(image);
        let panel = focus(&image, true);
        assert_eq!(panel.width(), 400);
        assert_eq!(panel.height(), 172);
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
}
