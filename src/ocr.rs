use std::{
    borrow::Cow,
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImage, ImageFormat, imageops::FilterType};

const MIN_RESULT_CONFIDENCE: f32 = 45.0;
const MIN_LINE_CONFIDENCE: f32 = 25.0;
const COLORED_TEXT_THRESHOLD: u8 = 179;
const TOP_HEIGHT_PERCENT: u32 = 30;
const BOTTOM_Y_PERCENT: u32 = 65;
const BOTTOM_HEIGHT_PERCENT: u32 = 25;
const EXTENDED_BOTTOM_HEIGHT_PERCENT: u32 = 33;

#[derive(Debug, Clone)]
pub struct OcrResult {
    pub text: String,
    pub confidence: f32,
    pub line_count: usize,
    pub word_confidences: Vec<f32>,
}

struct OcrLine {
    key: (u32, u32, u32),
    words: Vec<String>,
    confidence_total: f32,
    word_count: usize,
    word_confidences: Vec<f32>,
}

pub fn focus(image: &DynamicImage, detect_panel: bool) -> Cow<'_, DynamicImage> {
    if detect_panel {
        let top_height = image.height() * TOP_HEIGHT_PERCENT / 100;
        let bottom_y = image.height() * BOTTOM_Y_PERCENT / 100;
        let bottom_height = image.height() * EXTENDED_BOTTOM_HEIGHT_PERCENT / 100;
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
    let top_height = image.height() * TOP_HEIGHT_PERCENT / 100;
    let bottom_y = image.height() * BOTTOM_Y_PERCENT / 100;
    let bottom_height = image.height() * BOTTOM_HEIGHT_PERCENT / 100;
    let top = recognize_panel(&image.crop_imm(0, 0, image.width(), top_height));
    let standard_bottom = recognize_panel(&image.crop_imm(
        0,
        bottom_y,
        image.width(),
        bottom_height,
    ));
    let bottom = if standard_bottom.as_ref().is_ok_and(is_complete_candidate) {
        standard_bottom
    } else {
        let extended_height = image.height() * EXTENDED_BOTTOM_HEIGHT_PERCENT / 100;
        let extended = recognize_panel(&image.crop_imm(
            0,
            bottom_y,
            image.width(),
            extended_height,
        ));
        match (standard_bottom, extended) {
            (Ok(standard), Ok(extended)) => Ok(
                if dialogue_score(&standard) >= dialogue_score(&extended) {
                    standard
                } else {
                    extended
                },
            ),
            (Ok(result), Err(_)) | (Err(_), Ok(result)) => Ok(result),
            (Err(standard), Err(extended)) => {
                bail!("standard bottom: {standard:#}; extended bottom: {extended:#}")
            }
        }
    };
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

fn recognize_panel(image: &DynamicImage) -> Result<OcrResult> {
    let source = image.to_rgb8();
    let mut strongest_channel = image::GrayImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let brightest_channel = pixel.0.into_iter().max().unwrap_or_default();
        strongest_channel.put_pixel(x, y, image::Luma([brightest_channel]));
    }
    let primary = recognize_binary(strongest_channel);
    if primary.as_ref().is_ok_and(is_complete_candidate) {
        return primary;
    }

    let secondary = recognize_binary(image.grayscale().to_luma8());
    match (primary, secondary) {
        (Ok(primary), Ok(secondary)) => Ok(if dialogue_score(&primary) >= dialogue_score(&secondary)
        {
            primary
        } else {
            secondary
        }),
        (Ok(result), Err(_)) | (Err(_), Ok(result)) => Ok(result),
        (Err(primary), Err(secondary)) => {
            bail!("color candidate: {primary:#}; brightness candidate: {secondary:#}")
        }
    }
}

fn recognize_binary(mut image: image::GrayImage) -> Result<OcrResult> {
    let threshold = otsu_level(&image).min(COLORED_TEXT_THRESHOLD);
    for pixel in image.pixels_mut() {
        pixel[0] = if pixel[0] > threshold { 255 } else { 0 };
    }
    let image = DynamicImage::ImageLuma8(image);
    recognize_prepared(&image, true)
}

fn is_complete_candidate(result: &OcrResult) -> bool {
    result.confidence >= 55.0
        && result.text.split_whitespace().count() >= 3
        && result.text.ends_with(['.', '!', '?'])
}

fn otsu_level(image: &image::GrayImage) -> u8 {
    let mut histogram = [0_u64; 256];
    for pixel in image.pixels() {
        histogram[usize::from(pixel[0])] += 1;
    }
    let total = u64::from(image.width()) * u64::from(image.height());
    let weighted_total: u64 = histogram
        .iter()
        .enumerate()
        .map(|(level, count)| level as u64 * count)
        .sum();
    let mut background_count = 0_u64;
    let mut background_weight = 0_u64;
    let mut best_level = 0_u8;
    let mut best_variance = 0.0_f64;
    for (level, count) in histogram.iter().copied().enumerate() {
        background_count += count;
        if background_count == 0 || background_count == total {
            continue;
        }
        background_weight += level as u64 * count;
        let foreground_count = total - background_count;
        let background_mean = background_weight as f64 / background_count as f64;
        let foreground_mean = (weighted_total - background_weight) as f64 / foreground_count as f64;
        let variance = background_count as f64
            * foreground_count as f64
            * (background_mean - foreground_mean).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best_level = level as u8;
        }
    }
    best_level
}

fn dialogue_score(result: &OcrResult) -> f32 {
    let words = result.text.split_whitespace().count().min(10) as f32;
    let complete = f32::from(result.text.ends_with(['.', '!', '?'])) * 10.0;
    result.confidence + words * 3.0 + complete
}

fn validate_dialogue(mut result: OcrResult) -> Result<OcrResult> {
    trim_leading_artifacts(&mut result);
    let words: Vec<_> = result.text.split_whitespace().collect();
    let contains_url = words.iter().any(|word| {
        word.starts_with("http://") || word.starts_with("https://") || word.starts_with("www.")
    });
    let wordlike = words
        .iter()
        .filter(|word| word.chars().filter(|character| character.is_alphabetic()).count() >= 2)
        .count();
    let single_character_words = words
        .iter()
        .filter(|word| word.chars().filter(|character| character.is_alphanumeric()).count() <= 1)
        .count();
    let visible_characters = result
        .text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    let symbols = result
        .text
        .chars()
        .filter(|character| !character.is_whitespace() && !character.is_alphanumeric())
        .count();
    let incomplete_short_text = !result.text.ends_with(['.', '!', '?']) && wordlike < 4;
    let repeated_single_characters = single_character_words * 2 > words.len();
    let symbol_heavy = symbols * 3 > visible_characters;
    if result.line_count > 6
        || words.len() > 80
        || contains_url
        || wordlike < 2
        || incomplete_short_text
        || repeated_single_characters
        || symbol_heavy
    {
        bail!(
            "OCR result rejected as non-dialogue (lines={} words={} wordlike={wordlike} singles={single_character_words} symbols={symbols} url={contains_url})",
            result.line_count,
            words.len()
        );
    }
    Ok(result)
}

fn trim_leading_artifacts(result: &mut OcrResult) {
    let words: Vec<_> = result
        .text
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if words.is_empty() || matches!(words[0].as_str(), "I" | "A") {
        return;
    }
    let start = words.iter().position(|word| {
        word.chars().next().is_some_and(char::is_alphabetic)
            && word.chars().filter(|character| character.is_alphabetic()).count() >= 2
    });
    let Some(start) = start.filter(|start| *start > 0) else {
        return;
    };
    result.text = words[start..].join(" ");
    if result.word_confidences.len() == words.len() {
        result.word_confidences.drain(..start);
    }
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
                if original.word_confidences.len() == enlarged.word_confidences.len() {
                    merge_words(original, enlarged)
                } else {
                    enlarged
                }
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

fn merge_words(original: OcrResult, enlarged: OcrResult) -> OcrResult {
    let original_words = original.text.split_whitespace();
    let enlarged_words = enlarged.text.split_whitespace();
    let words: Vec<_> = original_words
        .zip(enlarged_words)
        .zip(
            original
                .word_confidences
                .iter()
                .zip(&enlarged.word_confidences),
        )
        .map(
            |((original, enlarged), (original_confidence, enlarged_confidence))| {
                let preserves_diacritics = !original.is_ascii() && enlarged.is_ascii();
                if !preserves_diacritics && enlarged_confidence > original_confidence {
                    enlarged
                } else {
                    original
                }
            },
        )
        .collect();
    let word_confidences: Vec<_> = original
        .word_confidences
        .iter()
        .zip(&enlarged.word_confidences)
        .map(|(original, enlarged)| original.max(*enlarged))
        .collect();
    let confidence = word_confidences.iter().sum::<f32>() / word_confidences.len() as f32;
    OcrResult {
        text: words.join(" "),
        confidence,
        line_count: original.line_count.max(enlarged.line_count),
        word_confidences,
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
            line.word_confidences.push(confidence);
        } else {
            lines.push(OcrLine {
                key,
                words: vec![text],
                confidence_total: confidence,
                word_count: 1,
                word_confidences: vec![confidence],
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
                && confidence >= MIN_LINE_CONFIDENCE)
                .then_some((
                    text,
                    line.confidence_total,
                    line.word_count,
                    line.word_confidences,
                ))
        })
        .collect();
    let count: usize = accepted.iter().map(|line| line.2).sum();
    let confidence = accepted.iter().map(|line| line.1).sum::<f32>() / count.max(1) as f32;
    let line_count = accepted.len();
    let word_confidences = accepted
        .iter()
        .flat_map(|line| line.3.iter().copied())
        .collect();
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
    validate_dialogue(OcrResult {
        text,
        confidence,
        line_count,
        word_confidences,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use image::{DynamicImage, Rgb, RgbImage};

    use super::{OcrResult, focus, otsu_level, parse_tsv, recognize, recognize_window, validate_dialogue};

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
    fn otsu_separates_text_from_dark_panel() {
        let mut image = image::GrayImage::from_pixel(100, 20, image::Luma([40]));
        for x in 20..80 {
            for y in 5..15 {
                image.put_pixel(x, y, image::Luma([220]));
            }
        }
        let threshold = otsu_level(&image);
        assert!((40..220).contains(&threshold));
    }

    #[test]
    fn rejects_urls_from_other_applications() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t90\tOpen\n\
5\t1\t1\t1\t1\t2\t0\t0\t1\t1\t90\thttps://example.com\n";
        assert!(parse_tsv(tsv).is_err());
    }

    #[test]
    fn rejects_dense_multiline_desktop_text() {
        let mut tsv = String::from(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n",
        );
        for line in 1..=7 {
            tsv.push_str(&format!(
                "5\t1\t1\t1\t{line}\t1\t0\t0\t1\t1\t90\tTerminal\n\
                 5\t1\t1\t1\t{line}\t2\t0\t0\t1\t1\t90\toutput\n"
            ));
        }
        assert!(parse_tsv(&tsv).is_err());
    }

    #[test]
    fn rejects_high_confidence_garbage_from_game_animation() {
        for text in ["T T T T T T T T", "I'_'II A I", "4% 3", "2 2"] {
            let words = text.split_whitespace().count();
            assert!(
                validate_dialogue(OcrResult {
                    text: text.into(),
                    confidence: 90.0,
                    line_count: 1,
                    word_confidences: vec![90.0; words],
                })
                .is_err(),
                "accepted {text}"
            );
        }
    }

    #[test]
    fn keeps_short_complete_game_dialogue() {
        for text in ["Go! Charmander!", "Are you lost?"] {
            let words = text.split_whitespace().count();
            assert!(
                validate_dialogue(OcrResult {
                    text: text.into(),
                    confidence: 70.0,
                    line_count: 1,
                    word_confidences: vec![70.0; words],
                })
                .is_ok(),
                "rejected {text}"
            );
        }
    }

    #[test]
    fn removes_leading_panel_artifacts() {
        for (input, expected) in [
            ("L 1 I Would you like to rest your Pokémon?", "Would you like to rest your Pokémon?"),
            ("0K, IT'll take your Pokémon for a few", "IT'll take your Pokémon for a few"),
            ("_ljl' a Charmander used Ember!", "Charmander used Ember!"),
        ] {
            let words = input.split_whitespace().count();
            let result = validate_dialogue(OcrResult {
                text: input.into(),
                confidence: 70.0,
                line_count: 1,
                word_confidences: vec![70.0; words],
            })
            .unwrap();
            assert_eq!(result.text, expected);
        }
    }

    #[test]
    fn selects_top_and_bottom_hud_bands_regardless_of_color() {
        let image = RgbImage::from_pixel(400, 400, Rgb([30, 30, 30]));
        let image = DynamicImage::ImageRgb8(image);
        let panel = focus(&image, true);
        assert_eq!(panel.width(), 400);
        assert_eq!(panel.height(), 252);
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
            "We restore your tired Pokémon to Tull health."
        );
    }

    #[test]
    fn recognizes_selected_text_in_bottom_dialogue_panel() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/medicine-pocket.png");
        let image = image::open(fixture).unwrap();
        assert_eq!(
            recognize_window(&image).unwrap().text,
            "You put the Potion amay in the ANedicine Pocket."
        );
    }

    #[test]
    fn recognizes_both_lines_near_the_window_bottom() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/trainer-battle-two-lines.png");
        let image = image::open(fixture).unwrap();
        let text = recognize_window(&image).unwrap().text;
        assert!(text.contains("When you're having trouble in a trainer"), "{text}");
        assert!(text.contains("allowed to forfeit"), "{text}");
    }

}
