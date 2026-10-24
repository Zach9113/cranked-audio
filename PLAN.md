# Cranked Audio: plan

Physical (or on-screen) knobs for Linux audio: one knob per group of apps, with buttons, lights and smart-home
control. Very light on the system: a tiny background service plus a window that only exists while it's open.

## What makes it different

- **Works without hardware.** The window's knobs and buttons are first-class controllers, not a preview.
- **Any controller, one set of rules.** Every input source produces the same knob/button events:
  - on-screen controls
  - 5-knob USB serial mixers (CH343-based, `SPEC.md` section 1)
  - planned: MIDI controllers (any knob/fader/pad), DIY Arduino slider boxes (newline-separated values over serial)
- **Native and lean.** Talks to PipeWire in-process (no helper processes per change), event-driven, no polling.
  Measured: 6 MB RAM and 0% CPU idle, 620 KB binary.
- **Rules that respect the user:** playback streams only (never the mic or screen-share), voice chat kept out of
  "everything else", apps that reset their own volume get put back, Auto-lower while someone talks.
- **Home Assistant built in:** a knob can be a light dimmer, a button can toggle anything.

## Architecture

```
cranked-audiod (service, always running, no window)         cranked-audio (window, only when opened)
  inputs: serial mixers, MIDI, on-screen (all -> events)     knobs, buttons, settings
  audio: libpulse (events, volumes, level meters)   <──── Unix socket, JSON lines ──────────>
  rules: matching, catch-all, voice guard, fast guard,      reads state, sends on-screen input,
         Auto-lower                                         edits settings
  Home Assistant: REST + event stream
  KDE popup + MPRIS (zbus), tray icon (ksni)
  settings: ~/.config/cranked-audio/config.toml
```

## Phases

1. **Spec + protocol** (done): `SPEC.md`, packet parser, knob filter, LED frames.
2. **Headless core** (done 2026-10-23, `crates/daemon`, binary `cranked-audiod`): serial, startup sync, knob filter,
   1% steps, stream matching, catch-all, fast guard, master volume, volume ranges, on-screen input (`--stdin`),
   `--dry-run`, `--print-config`. Unit tests + `tests/live.sh` (throwaway streams on a null sink).
3. **Buttons + feedback** (done 2026-10-23, first run on the real mixer OK): press/hold, mute, media keys (MPRIS,
   via playerctld when present), output switch (real devices only), KDE popups, LEDs, profiles (active one kept in
   `~/.local/state/cranked-audio/state.toml`). Not yet: screen-lock LED off, Home Assistant actions (phase 4).
4. **Home Assistant** (done 2026-10-23): light knobs with proportional catch-up (never moved by startup or profile
   switches), live state from the event stream (cheap text pre-check, only watched entities parsed) with polling
   while it's down, 150 ms throttle per light, button toggles with "Turning on ..." + result popups.
   `tests/live_ha.sh` runs it against `tests/fake_ha.py`.
   **Auto-lower** (done 2026-10-23): in-process peak meters (25 readings/s) only on trigger knobs' apps and voice
   chat, never a lowered knob; fade 0.25 s down, hold 1.2 s, 0.8 s back; knob turns, new streams and the guard
   respect it; LEDs dim with it. `tests/live_duck.sh` (tone vs silence on a null sink).
   The Rust app now does everything the old one did, except the window.
5. **Window** (first version 2026-10-23, `crates/window`, binary `cranked-audio`, Slint with the software renderer:
   25 MB while open, nothing when closed): live knobs (level arcs in the knob's colours, what's playing, muted /
   lowered), square buttons with icons and press/hold labels, drag/scroll knobs (Shift = 5%), press/hold
   buttons, Settings opens config.toml (applied live). Talks to the service over a Unix socket
   (`crates/ipc`) instead of D-Bus: simpler, same cost. Starts the service if it isn't running. Without a mixer the
   knobs start where they were (state.toml). Tray icon in the service (open, profiles, settings file, quit).
   Settings inside the window: right-click a knob (or click its name) for apps / output / HA light, range and
   colours; right-click a button for press/hold actions with HA entity pickers; Settings for profiles,
   Auto-lower, popups, LEDs and Home Assistant (token saved 0600). The service is the only writer of config.toml.
   Still to do: install + autostart (phase 7).
6. **More controllers**: MIDI, DIY serial sliders.
7. **Packaging**: `install.sh` (done 2026-10-24: ~/.local, app-menu entry, icon, systemd user service
   cranked-audio.service, `--uninstall`). Next: AUR, then Flatpak/.deb/.rpm; README with screenshots.

## License

GPL-3.0-or-later (chosen 2026-10-23): open source, and shared changes must stay open source.
