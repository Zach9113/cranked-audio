# Changelog

## Unreleased

### Development
- The workspace lint set (clippy pedantic plus most of restriction) is now switched on for every crate and clean.
  Warnings fail CI (new `CI` workflow: fmt, `clippy -D warnings`, unit tests) but not plain builds, so a newer Rust
  can't break source builds for users and packagers. A few pure-style rules are parked in `Cargo.toml` for now.
- Number conversions go through checked helpers (`cranked_protocol::convert`) that saturate instead of wrapping.
- No `unsafe` left: the keyboard knob waits for input with nix's safe `poll` instead of raw libc.

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
