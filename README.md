# game-translate

A small, fast English-to-Chinese game dialogue translator for Hyprland. It captures a user-selected Wayland region directly into memory, waits for the image to settle, runs Tesseract OCR, suppresses near-duplicate text, and translates new dialogue.

The first supported setup is Pokémon Infinite Fusion running under Proton on Hyprland.

## How it works

1. `slurp` selects either a dialogue region or an entire program window.
2. Hyprland IPC binds that selection to its source window, so tiled-window movement cannot redirect capture to another application.
3. `libwayshot` captures the translated region through the wlroots screencopy protocol without temporary screenshot files.
4. A sampled-pixel state machine waits for three stable frames, then a second OCR pass confirms that typewriter text is complete.
5. Tesseract TSV confidence rejects low-quality noise; normalized Levenshtein similarity suppresses OCR jitter and repeated dialogue.
6. Known battle templates use canonical Pokémon terminology; other dialogue is sent to Google Translate on a worker thread.

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

Region mode follows a text rectangle relative to the selected game window:

```sh
game-translate-toggle
```

Select only the dialogue text area and avoid the dialogue-box border.

Window mode follows one program window in its entirety, even when it moves, resizes, or changes workspace:

```sh
game-translate-toggle --window
```

Click the game window once. Capture pauses whenever that window is not on an active workspace, so another program can never replace it at the same screen coordinates.

## Test

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

The OCR regression tests invoke the system `tesseract` executable.

Runtime diagnostics are written to `~/.local/state/game-translate/game-translate.log`. The file is truncated on startup after it exceeds 1 MiB; screenshots are never logged.

## Scope

This version deliberately has one capture backend, one OCR engine, one language pair, and one output UI. It does not include legacy Python paths, fallback capture commands, migration code, or speculative configuration layers.

## License

MIT
