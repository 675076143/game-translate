use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageFormat};

pub fn recognize(image: &DynamicImage) -> Result<String> {
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .context("无法编码 OCR 输入")?;

    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", "eng", "--psm", "11"])
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
    Ok(normalize(&raw))
}

fn normalize(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| {
            line.chars()
                .filter(|character| character.is_alphanumeric())
                .count()
                >= 2
        })
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{normalize, recognize};

    #[test]
    fn joins_wrapped_dialogue() {
        assert_eq!(
            normalize("We restore your tired Pokémon to full\n\nhealth.\n"),
            "We restore your tired Pokémon to full health."
        );
    }

    #[test]
    fn recognizes_pokemon_center_dialogue() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pokemon-center.png");
        let image = image::open(fixture).unwrap();
        assert_eq!(
            recognize(&image).unwrap(),
            "We restore your tired Pokémon to full health."
        );
    }
}
