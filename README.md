# game-translate

A small, fast English-to-Chinese game dialogue translator for Hyprland. It captures a user-selected Wayland region directly into memory, waits for the image to settle, runs Tesseract OCR, suppresses near-duplicate text, and translates new dialogue.

The first supported setup is Pokémon Infinite Fusion running under Proton on Hyprland.

## How it works

1. `slurp` selects the dialogue text region.
2. `libwayshot` captures that region through the wlroots screencopy protocol without temporary screenshot files.
3. A sampled-pixel state machine waits for three stable frames.
4. Tesseract reads the stable image from standard input.
5. Normalized Levenshtein similarity suppresses OCR jitter and repeated dialogue.
6. A worker thread sends new English text to Google Translate and prints the result in a Kitty terminal.

## Requirements

- Hyprland with the wlroots screencopy protocol
- Rust 1.85 or newer
- `slurp`
- `tesseract` with English language data
- `kitty`, `jq`, and `hyprctl` for the included toggle script
- Internet access for translation

On Arch Linux:

```sh
sudo pacman -S --needed rust slurp tesseract tesseract-data-eng kitty jq
```

## Build and install

```sh
cargo build --release
install -Dm755 target/release/game-translate ~/.local/bin/game-translate
install -Dm755 game-translate-toggle ~/.local/bin/game-translate-toggle
```

Run `game-translate-toggle`, then select only the dialogue text area. Avoid including the dialogue-box border.

## Test

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

The OCR regression tests invoke the system `tesseract` executable.

## Scope

This version deliberately has one capture backend, one OCR engine, one language pair, and one output UI. It does not include legacy Python paths, fallback capture commands, migration code, or speculative configuration layers.

## License

MIT
