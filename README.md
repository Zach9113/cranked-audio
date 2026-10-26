# Cranked Audio

Volume knobs for Linux: give each group of apps its own knob, on a USB mixer or right on screen.

![Cranked Audio](data/screenshot.png)

## What it does

- **A knob per group of apps:** browsers and video on one knob, music on another, voice chat on a third, and
  "everything else" on a fourth. Apps are matched by name, so new windows and tabs follow their knob.
- **Works with or without hardware:** use a 5-knob USB serial mixer (CH343-based, with 15 RGB LEDs and 5 buttons),
  or drag and scroll the knobs in the window, the panel widget or with keyboard shortcuts. Without a mixer, knobs start
  at the volume that's already playing. Plug a mixer in later and it takes over.
- **Keyboard shortcuts (KDE Plasma):** every knob can be turned up, turned down and pressed (tap or hold) from the
  keyboard, plus next profile and open the window. Pick your keys in System Settings > Shortcuts > Cranked Audio.
- **As many knobs as you like:** 1 to 16 per profile, added with the + card and removed from a knob's menu. The mixer
  drives the first five; the rest live on screen or on other controllers.
- **More controllers:** a volume knob on your keyboard, any MIDI controller (click Learn and move a fader) or a
  deej-style Arduino slider box can turn knobs and press their buttons.
- **Buttons:** press and hold actions: mute a knob, play/pause, next/previous track, switch audio output, next profile,
  or toggle anything in Home Assistant.
- **Auto-lower:** turns music and videos down while voice chat or another knob is playing, then brings them back.
  Short sounds like notification pings don't count.
- **Keeps apps in line:** apps that reset their own volume (Spotify does on every song) are put back at once.
  Only playback is touched; microphones and screen sharing never are.
- **Knob lights:** each knob fills with its own two colours, follows the level smoothly, dims when nothing is playing
  and goes dark when the screen locks. Built-in calibration keeps dimmed colours true on your LEDs.
- **Your look:** dials, sliders or mixing-desk faders; themes (Cranked, Midnight, Graphite, Light, or your desktop's
  colours); size from tiny to large; choose which text shows, hide knobs, put the buttons inside the dials.
- **Atoll Dynamic Island integration:** Works natively with the [Atoll](https://github.com/Zach9113/atoll) dynamic island (`zach-patches` branch). Atoll shows volume feedback popups when knobs turn, and includes an embedded **Mixer** card in its expanded view to display, scroll, and drag your Cranked Audio knobs directly inside the island.
- **Popups where you want them:** KDE's on-screen display, the Atoll dynamic island, or standard
  notifications (automatic picks the island when it's running).
- **Panel widget (KDE Plasma):** your knobs right in a panel (sized to it; give it its own auto-hide panel to slide
  it in when you need it), on the desktop (with or without a background), or in a popup from a panel icon. Dials,
  sliders or faders; themes (match Plasma, Cranked, Glass, Minimal); choose which knobs and which text show.
  Right-click it to open the app, its settings or switch profile.
- **Scriptable:** a D-Bus API turns knobs, presses buttons and switches profiles from scripts, KDE Connect commands
  or automations.
- **Home Assistant:** a knob can dim a light, a button can toggle anything; changes made elsewhere show up live.
- **Profiles:** switch whole layouts (apps, colours, buttons, Auto-lower) from a button or the tray.
- **Light on the system:** a small background service (about 10 MB, near-zero CPU when idle) talks to PipeWire
  directly; the window only uses memory while it's open.

## Requirements

- Linux with PipeWire (with `pipewire-pulse`) or PulseAudio (Windows is an experimental preview, see
  [below](#windows-experimental-preview))
- systemd user services (for starting at login)
- KDE Plasma is recommended (popups, tray); other desktops get standard notifications and a tray icon where supported
- Optional: the USB mixer; Home Assistant

## Install

Build and install for your user (needs [Rust](https://rustup.rs) and the PulseAudio development files, e.g.
`libpulse` on Arch or `libpulse-dev` on Debian/Ubuntu):

```bash
git clone https://github.com/Zach9113/cranked-audio.git
cd cranked-audio
./install.sh
```

Or install a prebuilt release (no compiler needed; the download is checked against its published checksum):

```bash
curl -sSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-release.sh | bash
```

### Install with Atoll Dynamic Island (All-in-One)

To install both **Cranked Audio** and the **Atoll Dynamic Island** (with the in-island mixer card ready to go) in one command:

```bash
curl -fsSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-both.sh | bash
```

Both install to `~/.local` (no root needed), add Cranked Audio to your app menu and start it at every login.

To remove it (your settings stay in `~/.config/cranked-audio`):

```bash
./install.sh --uninstall
```

(or, for a prebuilt install: `curl -sSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-release.sh | bash -s -- --uninstall`)

Experimental: each release also has `CrankedAudio-x86_64.AppImage` to try it without installing (no automatic start
at login).

### Windows (experimental preview)

Cranked Audio is a Linux app first: everything above is made and tested on Linux, and that's where it's at home.
There's also an early Windows build. The service and window run on Windows 10 and 11 with per-app volume (through
Windows' own volume mixer), the tray icon, media keys, MIDI controllers and deej boxes. Popups, keyboard shortcuts,
keyboard knobs, the D-Bus API, the panel widget and in-app updates are Linux-only. From 2.1.0 on, each
release also has `cranked-audio-<version>-windows-x86_64-experimental.zip`: unzip it and run `install.ps1` in it. Or
build it with Rust (the MSVC toolchain), then install it for your user (Start menu entry, starts at sign-in, no
administrator needed):

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File install.ps1
```

`install.ps1 -Uninstall` removes it again (your settings stay in `%APPDATA%\cranked-audio`).

## Using it

Open **Cranked Audio** from the app menu or click the tray icon. Drag or scroll a knob to turn it; right-click a knob
to choose its apps, colours and range, and right-click a button to choose what a press and a hold do.
**Settings** covers the look, knobs, profiles, Auto-lower, popups, keyboard shortcuts, Home Assistant and
**Devices** (keyboard knobs, MIDI controllers and deej boxes). Mixer light settings (brightness, fill, calibration)
appear once a mixer has been connected.

For keyboard shortcuts, open **Settings > Set shortcuts…** (or System Settings > Shortcuts > Cranked Audio) and give
keys to the actions you want: "Knob 1: turn up", "Knob 1: button" and so on. None are set at first.

On KDE Plasma, right-click the panel or desktop, choose **Add Widgets…** and add **Cranked Audio**. In a panel it
shows the knobs themselves (a thin panel gets compact dials); its settings choose an icon with a popup instead.

### In-Island Mixer (Atoll Dynamic Island)

If you use the [Atoll](https://github.com/Zach9113/atoll) dynamic island, your Cranked Audio knobs can live right inside the island's expanded dashboard.

To install Atoll with the Cranked Audio integration:

```bash
git clone https://github.com/Zach9113/atoll.git
cd atoll
./scripts/install.sh
```

*(On Arch Linux / CachyOS, you can also run `makepkg -si` inside `packaging/`)*.

Once running, click Atoll to expand it and toggle **Mixer**: you get 270° dial gauges, custom LED colors, mute toggles, and smooth mouse-wheel volume adjustment synced with your mixer.

Everything is saved in `~/.config/cranked-audio/config.toml` (editing it by hand works too; changes apply at once).
The Home Assistant token is kept in its own file, readable only by you.

## Scripting

Cranked Audio answers on the session bus as `io.github.zach9113.CrankedAudio` (object
`/io/github/zach9113/CrankedAudio`, interface `io.github.zach9113.CrankedAudio1`):
`GetState`, `SetKnob(knob, percent)`, `StepKnob(knob, delta)`, `Press(button)`, `Hold(button)`, `SetProfile(name)`,
`OpenWindow`, `OpenSettings`, and a `StateChanged(json)` signal. Knobs and buttons count from 0. For example, to set the second knob
to 40%:

```bash
busctl --user call io.github.zach9113.CrankedAudio /io/github/zach9113/CrankedAudio io.github.zach9113.CrankedAudio1 SetKnob uu 1 40
```

## Development

If Nix is not installed, run the [official multi-user installer](https://nixos.org/download/):

```bash
curl --proto '=https' --tlsv1.2 -L https://nixos.org/nix/install | sh -s -- --daemon
```

Open a new terminal after installation, then [enable flakes](https://wiki.nixos.org/wiki/Flakes)
by adding this line to `~/.config/nix/nix.conf` (create the directory and file if needed):

```ini
experimental-features = nix-command flakes
```

With Nix, enter the pinned development shell on x86-64 or ARM64 Linux:

```bash
nix develop
```

The shell includes Rust, Cargo, Clippy, rustfmt, rust-analyzer, development tools, and the native
audio and windowing libraries, with Nushell available via `nu`. Use `nix develop -c <command>`
to run a command directly. For automatic activation with direnv and nix-direnv, run `direnv allow`.
The Nixpkgs and Fenix inputs share the pins from Meridian Client.

```bash
cargo build --release
target/release/cranked-audiod --dry-run -v     # show which app goes on which knob, change nothing
cargo test && tests/live.sh && tests/live_home_assistant.sh && tests/live_duck.sh && tests/live_dbus.sh
```

Before pushing, `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` must be
clean (CI runs both).

The live tests require a running PipeWire/PulseAudio server and a D-Bus session on the host.
The development shell supplies client tools; it does not start these services.

`SPEC.md` describes how everything behaves; `PLAN.md` is the roadmap.

## Made with AI

Cranked Audio was built with heavy use of AI coding assistants: most of the code, tests and documentation were
written by Claude (Anthropic, through Claude Code), with some work by Gemini (Google), under the direction of
Zach9113, who designed the features, made the decisions, and tested everything on real hardware. Commits that AI
helped write carry a `Co-Authored-By` line. Bug reports and reviews from humans are very welcome.

## License

Copyright (C) 2026 Zach9113. Free software under the GNU General Public License, version 3 or later; see `LICENSE`.
