# Cranked Audio

Knobs for your Linux audio: give each group of apps its own knob, on a USB mixer or on screen.

Early development. See `PLAN.md` for where it's going and `SPEC.md` for how it behaves.

## Install

### Option 1: Quick Install (No Rust or compiler required)

To install the latest prebuilt release directly to `~/.local`:
```bash
curl -sSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-release.sh | bash
```

To remove it later:
```bash
~/.local/bin/cranked-audio --uninstall # or run ./install.sh --uninstall
```

### Option 2: Standalone AppImage

Download `CrankedAudio-x86_64.AppImage` from [Releases](https://github.com/Zach9113/cranked-audio/releases), make it executable, and run:
```bash
chmod +x CrankedAudio-x86_64.AppImage
./CrankedAudio-x86_64.AppImage
```

### Option 3: Build from Source

```bash
git clone https://github.com/Zach9113/cranked-audio.git
cd cranked-audio
./install.sh               # builds, installs to ~/.local, and enables the login service
./install.sh --uninstall   # removes binaries and service (your settings stay)
```

Open **Cranked Audio** from the app menu or the tray icon. Right-click a knob or button on screen to change it.
Settings live in `~/.config/cranked-audio/config.toml`.

## Development

```
cargo build --release
target/release/cranked-audiod --dry-run -v     # show which app goes on which knob, change nothing
cargo test && tests/live.sh && tests/live_ha.sh && tests/live_duck.sh
```

## License

Copyright (C) 2026 Zach9113. Free software under the GNU General Public License, version 3 or later;
see `LICENSE`.
