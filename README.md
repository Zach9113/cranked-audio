# Cranked Audio

Knobs for your Linux audio: give each group of apps its own knob, on a USB mixer or on screen.

Early development. See `PLAN.md` for where it's going and `SPEC.md` for how it behaves.

## Install

```
./install.sh               # builds, installs to ~/.local, starts it, and starts it at every login
./install.sh --uninstall   # removes it again (your settings stay)
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
