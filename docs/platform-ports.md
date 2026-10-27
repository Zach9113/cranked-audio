# Platform ports and adapters

The daemon talks to the operating system in six areas. Today each area is `mod.rs` (shared types) plus `linux.rs` and
`windows.rs` behind `#[cfg]`, and the two files only *happen* to export the same functions. This plan turns each area
into a written-down **port** (a trait), with one **adapter** per platform behind a `Box<dyn …>`; only where the
shape is naturally a list (input devices) does it become a `Vec<Box<dyn …>>`. One function, `platform::native()`,
picks this OS's adapters at startup, so that is the only place in the daemon that knows which OS it's on.

Why:

- **One seam per area instead of `cfg` spread around.** The service, `Core` and the device logic never name an OS.
- **Testable.** A fake adapter (records commands, plays back scripted events) lets the service be tested without
  PipeWire, a desktop session or a Windows VM. Only the thin adapters still need the live suites.
- **`unsafe` stays in the adapters.** At the end the Windows adapters (all the COM and Win32 code) can move to their
  own crate, and the daemon crate can `forbid(unsafe_code)`.
- **Dynamic dispatch, not generics.** One adapter per platform, called at most a few dozen times a second: a vtable
  call costs nothing that matters here, and it keeps compile times and type signatures small (no `Service<A, D, T…>`).

## Ports

| Port (trait) | Adapters | What it covers today |
|---|---|---|
| `AudioSystem` | PipeWire/Pulse, WASAPI | `audio::run`, `audio::list_outputs` |
| `Desktop` | KDE/D-Bus, Win32 | media keys, volume popups, screen-off watch |
| `TrayIcon` | ksni, tray-icon | `tray::start`, `tray::Handle` updates |
| `GlobalShortcuts` | KDE kglobalaccel, `RegisterHotKey` | `shortcuts::Shortcuts` |
| `RemoteApi` | D-Bus, none on Windows | `dbus::start`, `dbus::publish` |
| `InputSource` | evdev / Raw Input keyboard knobs, MIDI, deej | `devices::start`: becomes a `Vec<Box<dyn InputSource>>`, one per configured device |

The worker threads and channels stay as they are: a port's `start` spawns its own thread, which speaks the existing
`Command`/`Event` messages, so the service owns each adapter outright (no shared `Arc`). The traits make that
contract explicit and swappable; they don't redesign it.

## Steps (one PR each, Linux behaviour unchanged, every check green on both targets)

1. **Audio.** `AudioSystem` trait (`start`, `outputs`), `platform::native()`, `Service::start` takes the `Platform`. *(this PR)*
2. **Desktop.** Media keys, popups and the screen watch behind `Desktop`.
3. **Tray.** `TrayIcon` with `start` and `update`.
4. **Shortcuts.** `GlobalShortcuts` with `start`/`ensure`.
5. **Remote API.** `RemoteApi` with `publish`; Windows plugs in a no-op adapter instead of `type Published = ((), ())`.
6. **Devices.** `InputSource` per device; MIDI and deej are portable and shared, keyboard knobs per platform.
7. **Layout and tests.** Move the adapters to `platform/linux/` and `platform/windows/` (the last `cfg` lines in
   one module), add a fake `Platform`, and test the service against it. Then decide whether the Windows adapters
   become their own crate so the daemon can `forbid(unsafe_code)`.
