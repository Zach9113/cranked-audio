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
| `TrayIcon` | ksni, tray-icon | `tray::start`, `tray::Handle` updates (done: `start`, `show`) |
| `GlobalShortcuts` | KDE kglobalaccel, `RegisterHotKey` | `shortcuts::Shortcuts` |
| `RemoteApi` | D-Bus, none on Windows | `dbus::start`, `dbus::publish` |
| `InputSource` + `InputHardware` | evdev / Raw Input keyboard knobs, MIDI, deej | `devices::start`: becomes a `Vec<Box<dyn InputSource>>`, one per configured device |

The worker threads and channels stay as they are: a port's `start` spawns its own thread, which speaks the existing
`Command`/`Event` messages, so the service owns each adapter outright (no shared `Arc`). The traits make that
contract explicit and swappable; they don't redesign it.

## Steps (one PR each, Linux behaviour unchanged, every check green on both targets)

1. **Audio.** `AudioSystem` trait (`start`, `outputs`), `platform::native()`, `Service::start` takes the `Platform`. *(done, #7)*
2. **Desktop.** Media keys, popups and the screen watch behind `Desktop` (`start`, `watch_screen`), adapters
   `linux::Plasma` and `windows::Win32`. *(#8)*
3. **Tray.** `TrayIcon` with `start` and `show`; adapters `linux::StatusNotifier` and `windows::NotificationArea`
   keep their own running icon. *(#9)*
4. **Shortcuts.** `GlobalShortcuts` with `start`/`ensure`; adapters `linux::KGlobalAccel` and `windows::HotKeys`.
   *(#10)*
5. **Remote API.** `RemoteApi` with `start` and `publish(&Snapshot)`; adapters `linux::SessionBus` and
   `windows::NotOffered` (a no-op instead of `type Published = ((), ())`). *(#11)*
6. **Devices.** `InputSource` per configured device, started from a `Vec<Box<dyn InputSource>>`; MIDI and deej
   sources are portable and shared. Keyboard knobs come from the `InputHardware` port (adapters `linux::Evdev`,
   `windows::RawInput`), which also lists keyboards and serial ports for Settings. *(#12)*
7. **Layout** *(7a, #13)*. Every adapter lives in `platform/linux/` or `platform/windows/`, one file per port
   (`audio`, `desktop`, `tray`, `shortcuts`, `remote_api`, `input_hardware`); the shared Win32 helpers
   (`attach_input_desktop`, `CRANKED_EXTRA_INFO`) sit in `platform/windows/mod.rs`. The six port modules have no
   `cfg` left; `platform/mod.rs` is the one place that picks the OS.
8. **Fake platform and service tests** *(7b, this PR)*. `platform::fake::platform()` gives a `Platform` whose
   adapters record what the service asked of them; `service::tests` start a real `Service` on it (temp settings,
   state and socket, no mixer) and check the commands and popups that come out. They run on Linux and Windows in
   milliseconds, with no `PipeWire`, desktop session or VM.

## Still OS-specific outside `platform/` (candidates for later ports)

- `serial.rs`, the mixer link: Windows needs a read timeout, DTR on open and LED frames written from the reader
  thread.
- `updater.rs`: Linux runs the release's `install.sh`, Windows swaps the two `.exe` files. Its Windows half
  (waiting for the old process with `OpenProcess`) is the last `unsafe` outside `platform/`; once it moves, the
  rest of the daemon can `forbid(unsafe_code)` without a separate crate.
- `main.rs`: one Windows block that waits for the old process after an update.
