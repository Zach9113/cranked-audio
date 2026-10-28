# Changelog

## Unreleased

### All platforms
- The tray menu has more in it: Settings…, Play/Pause, Next and Previous track, and Switch output, next to Open,
  your profiles, Edit settings file… and Quit.

### Linux
- The Plasma widget follows the knobs live: the service now offers its state as a D-Bus property (`State`, with
  `PropertiesChanged`), and the widget listens instead of asking every 5 seconds while it sat in the panel.
- Copies installed by a package manager (in `/usr`) no longer install updates themselves: Settings shows the new
  version and says to update with the package manager, instead of adding a second copy in `~/.local/bin`.

### Development
- Platform ports and adapters (plan: `docs/platform-ports.md`): the sound system is the first port, an
  `AudioSystem` trait with a PipeWire and a WASAPI adapter, picked once in `platform::native()`. The desktop
  (media keys, popups, screen-off watch) is the second: a `Desktop` trait with a Plasma and a Win32 adapter. The tray icon is the third: `TrayIcon` (`start`, `show`), and global shortcuts the fourth: `GlobalShortcuts`
  (`start`, `ensure`). The remote API is the fifth: `RemoteApi` (`start`, `publish`), D-Bus on Linux and a no-op
  adapter on Windows. Controllers are the sixth: each configured device runs as a `Box<dyn InputSource>`, keyboard
  knobs come from an `InputHardware` port (evdev, Raw Input), and a unit test runs a fake one without hardware. Every adapter now lives under `platform/linux/` or
  `platform/windows/`, and `platform/mod.rs` is the one place that picks the OS. A
  fake platform records what the service asks of each port, and the first service tests run on it. Updates are a
  port too (`Updater`: install.sh on Linux, the .exe swap on Windows), which moved the last `unsafe` code into the
  adapters: every other module of the service is now `#[forbid(unsafe_code)]`. A new live suite, `tests/live_update.sh`, updates a test copy
  from a fake release on localhost, including one whose download doesn't match its checksum.
- AUR package `cranked-audio-bin` (repackages the release tarball): `packaging/aur/cranked-audio-bin`, and
  `scripts/update-aur.sh vX.Y.Z` points it at a new release.
- The workspace lint set (clippy pedantic plus most of restriction) is now switched on for every crate and clean.
  Warnings fail CI (new `CI` workflow: fmt, `clippy -D warnings`, unit tests) but not plain builds, so a newer Rust
  can't break source builds for users and packagers. A few pure-style rules are parked in `Cargo.toml` for now.
- Number conversions go through checked helpers (`cranked_protocol::convert`) that saturate instead of wrapping.
- No `unsafe` left in the Linux code: the keyboard knob waits for input with nix's safe `poll` instead of raw libc.
- The Windows-only modules are lint-clean too (every COM/Win32 call in its own `unsafe` block with a SAFETY note),
  and CI lints them on a Windows runner.

## 2.2.0

### Linux
- Knobs past the mixer's five (a keyboard knob, MIDI, on-screen) start at the volume that's already playing, also
  while the mixer is connected.
- Switch output: virtual outputs are skipped unless you include them, and each output can be switched on or off
  for the cycle (Settings, and a button's page when it switches outputs).
- OLED theme (pure black) for the window and the panel widget.
- A knob with a volume range shows the app's real volume and marks its limits on the dial ("min 10%", "max 60%").
- Clicking the tray icon or Open Cranked Audio brings an open window to the front.

### Windows (experimental preview)
Tested on the real mixer this time.
- The mixer reports every knob at start, and its lights stay steady (no more flashing).
- Knobs no longer fight their own volume changes; play/pause controls the media player that's playing; holding a
  button to mute unmutes again on the next hold.
- New on Windows: keyboard knob, on-screen popups, screen sleep and lock turning the lights off, shortcuts (set in
  Settings), and installing updates from the app (Settings > About; checked against the release's checksum).
- Output switching skips virtual outputs such as Steam Streaming Speakers.
- Still unsigned: Smart App Control blocks it where it's on.

## 2.1.1

- Fixed: a second copy of the service (another start, or a test run) took over the D-Bus name and left the panel
  widget and Atoll's mixer blank once it exited. The running service now keeps its name.
- Fixed: the service could crash (and restart) when Auto-lower stopped watching an app's level, because the level
  meter was freed while PipeWire's thread was still using it.

## 2.1.0

### Knobs and controllers
- A knob's settings page picks its controller directly (mixer, keyboard knob, MIDI controller or deej slider).
- Better controller detection: rotary dials and wheels count as knob controllers, their buttons press the knob's
  button, and virtual devices, monitor audio jacks, microphones and system buttons are left out of the list.
- Fixed: saving a knob's page lost its learned MIDI control and moved its deej slider to the end of the box.
- Removing a knob asks first.

### Looks
- Buttons can sit inside the dials, next to the percentage, for a compact view.
- The add-knob button moved to the header.
- Settings read the same in the window, the panel widget and Atoll's mixer: Style, Theme, Size, Text, Buttons,
  Knobs shown; shortcuts, knobs and devices under "Knobs and controllers".
- Panel widget: the widget background and the cards behind the knobs can each be turned off (in a panel of its own
  that also clears the panel's frame); cards are darker with crisp borders so text stays readable over the
  wallpaper.

### Other
- D-Bus API: `NextProfile`.
- `install-both.sh` installs Cranked Audio and the Atoll dynamic island in one go (Atoll as a pacman package on
  Arch).
- Nix: a flake and dev shell.

### Windows (experimental preview)
Cranked Audio is still a Linux app first; this is an early Windows build for people who want to try it.
- Download `cranked-audio-v2.1.0-windows-x86_64-experimental.zip`, unzip it and run
  `powershell -ExecutionPolicy Bypass -File install.ps1` in it (Start menu entry, starts at sign-in, no
  administrator needed; `install.ps1 -Uninstall` removes it and keeps your settings).
- Works: per-app volume through Windows' own volume mixer (the same percentages it shows), the System knob, mute,
  Auto-lower, media keys, the tray icon, the USB mixer, MIDI controllers and deej boxes.
- Linux-only for now: popups, keyboard shortcuts, keyboard knobs, the D-Bus API, the panel widget and installing
  updates from the app.
- Nothing about the Linux app changed for it.

## 2.0.1

- Panel widget: the hover tooltip listing every knob's level is now optional when the knobs are shown in the panel
  (off by default; turn it on in Configure Widget > In a panel, handy when the knobs are small). The icon version
  keeps it.

## 2.0.0

Cranked Audio now works the way you want with or without the mixer.

### Knobs and controllers
- 1 to 16 knobs per profile: add one with the + card, remove one from its right-click menu. The mixer drives the
  first five.
- More controllers (Settings > Devices): a volume knob on a keyboard, any MIDI controller (click Learn and move a
  fader or press a pad) and deej-style Arduino slider boxes.
- Keyboard shortcuts on KDE Plasma for every knob (turn up, turn down, button: tap = press, hold = hold), next
  profile and opening the window. Set the keys in System Settings > Shortcuts > Cranked Audio.
- Without a mixer, knobs start at the volume that's already playing, and knobs beyond the mixer's five remember
  where they were.

### Panel widget (KDE Plasma)
- Knobs right in a panel, sized to it (compact dials in a thin panel). Give them their own auto-hide or
  dodge-windows panel to slide them in when you need them.
- On the desktop with or without a background, or as a popup from a panel icon.
- Dials, sliders or faders; themes; choose which knobs and text show; buttons inside the dials.
- Right-click menu: open the app, open its settings, switch profile, configure the widget.
- Settings split into Look, What shows, In a panel and Desktop & popup.

### The window
- Styles: dials, sliders or faders; themes (Cranked, Midnight, Graphite, Light, or your desktop's colours); any size;
  choose which text shows; hide knobs; buttons inside the dials.
- A one-time tip for new users; mixer light settings only appear once a mixer has been connected.

### Popups and scripting
- Popups can go to the Atoll dynamic island (or KDE's on-screen display, or notifications).
- A D-Bus API for scripts, KDE Connect and automations: GetState, SetKnob, StepKnob, Press, Hold, SetProfile,
  OpenWindow, OpenSettings and a StateChanged signal.

### Mixer lights
- LED fill styles: the classic Turn Up-style fade (default), a smooth blend, or only your two colours.
- Per-colour LED calibration so dimmed colours stay true.

### Fixes
- Auto-lower ignores short sounds like notification pings; talking still lowers right away.
- Restarting the service never turns on a Home Assistant light by itself.
- The updater checks every download against its published checksum.

## 1.0.0

First release.
