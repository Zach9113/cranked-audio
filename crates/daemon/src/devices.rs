//! Controllers besides the mixer: keyboard knobs (evdev), MIDI controllers and deej-style Arduino slider boxes.
//! Each runs on its own thread and turns what it reads into the same knob and button input as everything else.
//! Threads stop once `generation` moves past theirs (new settings started replacements).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cranked_ipc::settings::{Device, MidiControl};

use crate::desktop::MediaKey;
use crate::Event;

/// What a device did, in knob terms (knobs count from 0 here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Set { knob: usize, percent: u8 },
    Step { knob: usize, delta: i32 },
    Button { knob: usize, down: bool },
    Media(MediaKey),
}

/// A MIDI control the learn mode heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    pub port: String,
    pub channel: u8,
    pub kind: String,
    pub number: u8,
}

const RETRY: Duration = Duration::from_secs(3);

pub fn start(devices: &[Device], events: &Sender<Event>, generation: &Arc<AtomicU64>) {
    let mine = generation.fetch_add(1, Ordering::Relaxed) + 1;
    for device in devices.iter().cloned() {
        let (events, generation) = (events.clone(), Arc::clone(generation));
        let current = move || generation.load(Ordering::Relaxed) == mine;
        let _ = std::thread::Builder::new().name("device".into()).spawn(move || match device {
            Device::KeyboardKnob { name, knob, step, grab } => keyboard(&name, knob, step, grab, &events, &current),
            Device::Midi { name, controls } => midi(&name, &controls, &events, &current),
            Device::Deej { port, baud, knobs, invert } => deej(&port, baud, &knobs, invert, &events, &current),
        });
    }
}

fn send(events: &Sender<Event>, input: Input) -> bool {
    events.send(Event::Device(input)).is_ok()
}

// ---------------------------------------------------------------- keyboard knobs & controllers

/// Whether an input device types (has letter keys): those are never taken over.
fn types(device: &evdev::Device) -> bool {
    device.supported_keys().is_some_and(|k| k.contains(evdev::KeyCode::KEY_A) || k.contains(evdev::KeyCode::KEY_SPACE))
}

/// Whether an input device is a mouse or pointer (has relative X and Y movement).
pub fn is_pointer(device: &evdev::Device) -> bool {
    device.supported_relative_axes().is_some_and(|axes| {
        axes.contains(evdev::RelativeAxisCode::REL_X) && axes.contains(evdev::RelativeAxisCode::REL_Y)
    })
}

/// Substrings in device names that indicate virtual injectors, monitor audio jacks, microphones or system buttons
/// that shouldn't appear as knob candidates.
pub fn is_bad_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    const BAD: &[&str] = &[
        "cranked audio",
        "passthrough",
        "action injector",
        "virtual",
        "uinput",
        "ydotool",
        "xdotool",
        "monitor headset",
        "generic lg monitor",
        "hdmi/dp",
        "hda intel",
        "hda nvidia",
        "power button",
        "sleep button",
        "video bus",
        "pc speaker",
        "mystic light",
        "microphone",
        "webcam",
        "headset mic",
    ];
    // Realtek codec jack endpoints ("ALC1220 Headphone"), matched as a word so names like "Falcon" stay
    let realtek = lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        word.strip_prefix("alc").is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    });
    realtek || BAD.iter().any(|b| lower.contains(b))
}

/// Whether an evdev device is a bogus candidate (virtual device, monitor audio jack, microphone, system button).
pub fn is_bad_device(device: &evdev::Device) -> bool {
    if let Some(name) = device.name() {
        if is_bad_name(name) {
            return true;
        }
    }
    // ALSA physical path indicates an ALSA jack detection endpoint, not a USB/BT controller
    if device.physical_path().is_some_and(|p| p == "ALSA") {
        return true;
    }
    false
}

/// Whether an input device can act as a knob or audio controller.
/// Matches devices with volume keys, rotary dials (`REL_DIAL`), and standalone wheel devices (not mice).
pub fn has_knob_capabilities(device: &evdev::Device) -> bool {
    if is_bad_device(device) {
        return false;
    }

    let has_volume_keys = device
        .supported_keys()
        .is_some_and(|k| k.contains(evdev::KeyCode::KEY_VOLUMEUP) || k.contains(evdev::KeyCode::KEY_VOLUMEDOWN));

    let has_dial =
        device.supported_relative_axes().is_some_and(|axes| axes.contains(evdev::RelativeAxisCode::REL_DIAL));

    let has_standalone_wheel = !is_pointer(device)
        && device.supported_relative_axes().is_some_and(|axes| {
            axes.contains(evdev::RelativeAxisCode::REL_WHEEL) || axes.contains(evdev::RelativeAxisCode::REL_HWHEEL)
        });

    has_volume_keys || has_dial || has_standalone_wheel
}

/// Input devices that have volume keys or rotary dials/wheels (candidates for a knob controller), by name,
/// and which of them also type.
/// Non-typing devices (dedicated consumer control endpoints and rotary dials) come first so they are easily
/// chosen and can be taken over (grabbed) without breaking typing.
pub fn keyboard_knobs() -> (Vec<String>, Vec<String>) {
    let mut found: Vec<(bool, String)> = evdev::enumerate()
        .filter(|(_, d)| has_knob_capabilities(d))
        .filter_map(|(_, d)| d.name().map(|n| (types(&d), n.to_string())))
        .collect();

    found.sort();

    let mut seen = std::collections::HashSet::new();
    let mut names = Vec::new();
    let mut non_typing = std::collections::HashSet::new();
    let mut typing_candidates = std::collections::HashSet::new();

    for (is_typing, name) in found {
        if is_typing {
            typing_candidates.insert(name.clone());
        } else {
            non_typing.insert(name.clone());
        }
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    let typing: Vec<String> =
        names.iter().filter(|n| typing_candidates.contains(*n) && !non_typing.contains(*n)).cloned().collect();

    (names, typing)
}

/// What a key does on a keyboard knob (`value`: 1 down, 2 repeat, 0 up). Media keys only matter when the input is
/// taken over (otherwise the desktop already handles them).
pub fn keyboard_input(code: evdev::KeyCode, value: i32, knob: usize, step: u8, grabbed: bool) -> Option<Input> {
    use evdev::KeyCode as K;
    let step = i32::from(step.max(1));
    match (code, value) {
        (K::KEY_VOLUMEUP, 1 | 2) => Some(Input::Step { knob, delta: step }),
        (K::KEY_VOLUMEDOWN, 1 | 2) => Some(Input::Step { knob, delta: -step }),
        // Knob push-buttons: KEY_MUTE or controller buttons (BTN_0, BTN_1, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA)
        (K::KEY_MUTE | K::BTN_0 | K::BTN_1 | K::BTN_MIDDLE | K::BTN_SIDE | K::BTN_EXTRA, 1) => {
            Some(Input::Button { knob, down: true })
        }
        (K::KEY_MUTE | K::BTN_0 | K::BTN_1 | K::BTN_MIDDLE | K::BTN_SIDE | K::BTN_EXTRA, 0) => {
            Some(Input::Button { knob, down: false })
        }
        (K::KEY_PLAYPAUSE, 1) if grabbed => Some(Input::Media(MediaKey::PlayPause)),
        (K::KEY_NEXTSONG, 1) if grabbed => Some(Input::Media(MediaKey::Next)),
        (K::KEY_PREVIOUSSONG, 1) if grabbed => Some(Input::Media(MediaKey::Previous)),
        _ => None,
    }
}

/// What a relative axis movement does on a knob controller (rotary encoders, dials, and wheels).
pub fn relative_input(axis: evdev::RelativeAxisCode, value: i32, knob: usize, step: u8) -> Option<Input> {
    use evdev::RelativeAxisCode as R;
    let step = i32::from(step.max(1));
    match axis {
        R::REL_DIAL | R::REL_WHEEL | R::REL_HWHEEL => {
            if value == 0 {
                None
            } else {
                Some(Input::Step { knob, delta: value * step })
            }
        }
        _ => None,
    }
}

/// Waits up to `timeout` for the device to have something to read.
fn readable(fd: std::os::fd::RawFd, timeout: Duration) -> bool {
    let mut poll = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd for the duration of the call
    let ready = unsafe { libc::poll(&mut poll, 1, timeout.as_millis() as i32) };
    ready > 0 && poll.revents & libc::POLLIN != 0
}

/// Finds the best matching evdev node for a configured knob device name.
/// 1. Finds all device nodes that match knob capabilities.
/// 2. Matches the exact name if possible; if not found, the same name in a different capitalisation.
/// 3. If multiple nodes match, prefers non-typing nodes so grab succeeds and typing is not interrupted.
pub fn find_knob_device(name: &str) -> Option<(std::path::PathBuf, evdev::Device)> {
    let mut candidates: Vec<(std::path::PathBuf, evdev::Device)> =
        evdev::enumerate().filter(|(_, d)| has_knob_capabilities(d)).filter(|(_, d)| d.name() == Some(name)).collect();

    if candidates.is_empty() {
        // only a different capitalisation counts: a partial name could pick up an unrelated device
        // ("Consumer Control") while the configured one is unplugged
        candidates = evdev::enumerate()
            .filter(|(_, d)| has_knob_capabilities(d))
            .filter(|(_, d)| d.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .collect();
    }

    // Prefer non-typing endpoints (false before true)
    candidates.sort_by_key(|(_, d)| types(d));
    candidates.into_iter().next()
}

fn keyboard(name: &str, knob: usize, step: u8, grab: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    use std::os::fd::AsRawFd;
    let knob = knob.saturating_sub(1);
    let mut reported = false;
    while current() {
        let found = find_knob_device(name);
        let Some((path, mut device)) = found else {
            if !reported {
                eprintln!("keyboard knob: {name:?} not found; waiting for it");
                reported = true;
            }
            std::thread::sleep(RETRY);
            continue;
        };
        // never take over a device that types: the keyboard would stop working
        let types = types(&device);
        if grab && types {
            eprintln!("keyboard knob: {name} also types, so it isn't taken over (the system volume moves too)");
        }
        let grabbed = grab && !types && device.grab().is_ok();
        eprintln!(
            "keyboard knob: {name} ({}) on knob {}{}",
            path.display(),
            knob + 1,
            if grabbed { ", taken over" } else { "" }
        );
        reported = false;
        let fd = device.as_raw_fd();
        'read: while current() {
            if !readable(fd, Duration::from_secs(1)) {
                continue;
            }
            let Ok(batch) = device.fetch_events() else { break 'read };
            for event in batch {
                match event.destructure() {
                    evdev::EventSummary::Key(_, code, value) => {
                        if let Some(input) = keyboard_input(code, value, knob, step, grabbed) {
                            if !send(events, input) {
                                return;
                            }
                        }
                    }
                    evdev::EventSummary::RelativeAxis(_, axis, value) => {
                        if let Some(input) = relative_input(axis, value, knob, step) {
                            if !send(events, input) {
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if grabbed {
            let _ = device.ungrab();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ---------------------------------------------------------------- MIDI

/// Status byte, data, data -> (channel 1..16, "cc"/"note", number, value); note off = value 0.
pub fn midi_message(message: &[u8]) -> Option<(u8, &'static str, u8, u8)> {
    let (&status, rest) = message.split_first()?;
    let channel = (status & 0x0F) + 1;
    match (status & 0xF0, rest) {
        (0xB0, [number, value, ..]) => Some((channel, "cc", *number, *value)),
        (0x90, [number, velocity, ..]) => Some((channel, "note", *number, *velocity)),
        (0x80, [number, _, ..]) => Some((channel, "note", *number, 0)),
        _ => None,
    }
}

/// What a MIDI message does under the mapped controls.
pub fn midi_input(controls: &[MidiControl], message: &[u8]) -> Option<Input> {
    let (channel, kind, number, value) = midi_message(message)?;
    let control = controls.iter().find(|c| c.channel == channel && c.kind == kind && c.number == number)?;
    let knob = control.knob.saturating_sub(1);
    if control.button {
        return Some(Input::Button { knob, down: value > 0 });
    }
    if control.relative {
        // endless encoders: 1..63 turns up that many steps, 65..127 turns down
        let delta = if value < 64 { i32::from(value) } else { i32::from(value) - 128 };
        return (delta != 0).then_some(Input::Step { knob, delta });
    }
    Some(Input::Set { knob, percent: ((u32::from(value) * 100 + 63) / 127) as u8 })
}

pub fn midi_ports() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new("Cranked Audio") else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).filter(|n| !n.contains("Cranked Audio")).collect()
}

/// A port's name without ALSA's client:port numbers ("nanoKONTROL2:nanoKONTROL2 _ CTRL 20:0" loses " 20:0"),
/// which change from boot to boot, so a saved name keeps matching.
pub fn stable_port_name(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((rest, numbers))
            if numbers.split_once(':').is_some_and(|(a, b)| {
                !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
            }) =>
        {
            rest
        }
        _ => name,
    }
}

/// MIDI controllers to offer: ALSA's loopback ("Midi Through") isn't one.
pub fn midi_controllers() -> Vec<String> {
    midi_ports().iter().filter(|n| !n.starts_with("Midi Through")).map(|n| stable_port_name(n).to_string()).collect()
}

fn midi(name: &str, controls: &[MidiControl], events: &Sender<Event>, current: &dyn Fn() -> bool) {
    let mut reported = false;
    while current() {
        let Ok(input) = midir::MidiInput::new("Cranked Audio") else {
            std::thread::sleep(RETRY);
            continue;
        };
        let port = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n.contains(name)));
        let Some(port) = port else {
            if !reported {
                eprintln!("midi: no port named like {name:?}; waiting for it");
                reported = true;
            }
            std::thread::sleep(RETRY);
            continue;
        };
        let port_name = input.port_name(&port).unwrap_or_default();
        let (sender, controls_for_callback) = (events.clone(), controls.to_vec());
        let connection = input.connect(
            &port,
            "Cranked Audio",
            move |_, message, _| {
                if let Some(input) = midi_input(&controls_for_callback, message) {
                    let _ = sender.send(Event::Device(input));
                }
            },
            (),
        );
        let Ok(connection) = connection else {
            std::thread::sleep(RETRY);
            continue;
        };
        eprintln!("midi: {port_name}");
        reported = false;
        // keep the connection while the port exists and the settings haven't changed
        while current() && midi_ports().iter().any(|p| p == &port_name) {
            std::thread::sleep(Duration::from_secs(2));
        }
        drop(connection);
    }
}

/// Listens on every MIDI port for up to `wait` and reports the first knob, fader, button or pad that moves.
pub fn learn_midi(events: Sender<Event>, wait: Duration) {
    let Ok(probe) = midir::MidiInput::new("Cranked Audio") else { return };
    let ports = probe.ports();
    let (found, heard) = std::sync::mpsc::channel::<Learned>();
    let mut connections = Vec::new();
    for port in ports {
        let Ok(input) = midir::MidiInput::new("Cranked Audio learn") else { continue };
        let port_name = input.port_name(&port).unwrap_or_default();
        if port_name.contains("Cranked Audio") || port_name.starts_with("Midi Through") {
            continue;
        }
        let port_name = stable_port_name(&port_name).to_string();
        let found = found.clone();
        if let Ok(connection) = input.connect(
            &port,
            "Cranked Audio learn",
            move |_, message, _| {
                if let Some((channel, kind, number, _)) = midi_message(message) {
                    let _ = found.send(Learned { port: port_name.clone(), channel, kind: kind.into(), number });
                }
            },
            (),
        ) {
            connections.push(connection);
        }
    }
    let learned = heard.recv_timeout(wait).ok();
    drop(connections);
    let _ = events.send(Event::MidiLearned(learned));
}

// ---------------------------------------------------------------- deej

/// One line from a deej box ("512|1023|0") as percentages.
pub fn deej_line(line: &str, invert: bool) -> Vec<Option<u8>> {
    line.trim()
        .split('|')
        .map(|v| v.trim().parse::<u32>().ok().filter(|v| *v <= 1023))
        .map(|raw| {
            raw.map(|raw| {
                let percent = ((raw * 100 + 511) / 1023) as u8;
                if invert {
                    100 - percent
                } else {
                    percent
                }
            })
        })
        .collect()
}

/// Serial ports a deej box could be on.
/// USB vendors whose serial ports are boards a deej box is built on: Arduino, Arduino clones' USB chips (CH340,
/// FTDI, CP210x, PL2303), Adafruit, SparkFun, Teensy, Raspberry Pi (Pico) and Espressif (ESP32).
const BOARD_VENDORS: [&str; 11] =
    ["2341", "2a03", "1a86", "0403", "10c4", "067b", "239a", "1b4f", "16c0", "2e8a", "303a"];

pub fn is_board(vendor: &str) -> bool {
    BOARD_VENDORS.contains(&vendor.trim())
}

/// The USB vendor id of a serial device (a /dev path or a /dev/serial/by-id link).
fn usb_vendor(path: &std::path::Path) -> Option<String> {
    let tty = std::fs::canonicalize(path).ok()?;
    let name = tty.file_name()?.to_str()?;
    std::fs::read_to_string(format!("/sys/class/tty/{name}/device/../idVendor")).ok()
}

/// Serial ports that could be a deej box: USB boards only (not, say, a monitor's control port), and not the mixer.
pub fn serial_ports() -> Vec<String> {
    let mut ports: Vec<String> = std::fs::read_dir("/dev/serial/by-id")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| usb_vendor(&e.path()).is_some_and(|v| is_board(&v)))
        .map(|e| e.path().to_string_lossy().into_owned())
        // the TurnUp-style mixer is handled separately
        .filter(|p| !p.contains("usb-1a86_USB_Single_Serial_"))
        .collect();
    ports.sort();
    ports
}

fn deej(port: &str, baud: u32, knobs: &[usize], invert: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    use std::io::{BufRead, BufReader};
    let mut reported = false;
    while current() {
        let opened = serialport::new(port, baud).timeout(Duration::from_secs(1)).open();
        let Ok(serial) = opened else {
            if !reported {
                eprintln!("deej: can't open {port}; trying again");
                reported = true;
            }
            std::thread::sleep(RETRY);
            continue;
        };
        eprintln!("deej: {port}");
        reported = false;
        // last value sent per slider; pots jitter, so a slider must move 2% (or reach an end) to count
        let mut last: Vec<Option<u8>> = vec![None; knobs.len()];
        let mut reader = BufReader::new(serial);
        let mut line = String::new();
        let mut last_data = Instant::now();
        while current() {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    last_data = Instant::now();
                    for (i, value) in deej_line(&line, invert).into_iter().enumerate().take(knobs.len()) {
                        let Some(percent) = value else { continue };
                        let moved = last[i].is_none_or(|p| {
                            p.abs_diff(percent) >= 2 || (percent != p && (percent == 0 || percent == 100))
                        });
                        if moved {
                            last[i] = Some(percent);
                            if !send(events, Input::Set { knob: knobs[i].saturating_sub(1), percent }) {
                                return;
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // deej boxes send all the time; silence this long means it's gone
                    if last_data.elapsed() > Duration::from_secs(10) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::KeyCode as K;

    #[test]
    fn keyboard_knob_keys() {
        assert_eq!(keyboard_input(K::KEY_VOLUMEUP, 1, 3, 2, true), Some(Input::Step { knob: 3, delta: 2 }));
        assert_eq!(
            keyboard_input(K::KEY_VOLUMEDOWN, 2, 3, 2, true),
            Some(Input::Step { knob: 3, delta: -2 }),
            "repeat"
        );
        assert_eq!(keyboard_input(K::KEY_VOLUMEUP, 0, 3, 2, true), None, "release does nothing");
        assert_eq!(keyboard_input(K::KEY_MUTE, 1, 3, 2, true), Some(Input::Button { knob: 3, down: true }));
        assert_eq!(keyboard_input(K::KEY_MUTE, 0, 3, 2, true), Some(Input::Button { knob: 3, down: false }));
        // Controller push buttons (BTN_0, BTN_MIDDLE, etc.)
        assert_eq!(keyboard_input(K::BTN_0, 1, 3, 2, true), Some(Input::Button { knob: 3, down: true }));
        assert_eq!(keyboard_input(K::BTN_0, 0, 3, 2, true), Some(Input::Button { knob: 3, down: false }));
        assert_eq!(keyboard_input(K::BTN_MIDDLE, 1, 3, 2, true), Some(Input::Button { knob: 3, down: true }));
        assert_eq!(keyboard_input(K::KEY_PLAYPAUSE, 1, 3, 2, true), Some(Input::Media(MediaKey::PlayPause)));
        assert_eq!(keyboard_input(K::KEY_PLAYPAUSE, 1, 3, 2, false), None, "not taken over: the desktop handles it");
        assert_eq!(keyboard_input(K::KEY_A, 1, 3, 2, true), None);
    }

    #[test]
    fn relative_knob_inputs() {
        use evdev::RelativeAxisCode as R;
        // REL_DIAL
        assert_eq!(relative_input(R::REL_DIAL, 1, 2, 3), Some(Input::Step { knob: 2, delta: 3 }));
        assert_eq!(relative_input(R::REL_DIAL, -1, 2, 3), Some(Input::Step { knob: 2, delta: -3 }));
        assert_eq!(relative_input(R::REL_DIAL, 2, 2, 3), Some(Input::Step { knob: 2, delta: 6 }));
        assert_eq!(relative_input(R::REL_DIAL, 0, 2, 3), None);

        // REL_WHEEL and REL_HWHEEL
        assert_eq!(relative_input(R::REL_WHEEL, 1, 1, 2), Some(Input::Step { knob: 1, delta: 2 }));
        assert_eq!(relative_input(R::REL_WHEEL, -1, 1, 2), Some(Input::Step { knob: 1, delta: -2 }));
        assert_eq!(relative_input(R::REL_HWHEEL, 1, 1, 2), Some(Input::Step { knob: 1, delta: 2 }));

        // Other axes like X/Y or high-res wheels should be ignored
        assert_eq!(relative_input(R::REL_X, 10, 1, 2), None);
        assert_eq!(relative_input(R::REL_Y, 10, 1, 2), None);
        assert_eq!(relative_input(R::REL_WHEEL_HI_RES, 120, 1, 2), None);
    }

    #[test]
    fn device_name_filtering() {
        // Bad devices
        assert!(is_bad_name("Generic LG Monitor Headset MIC Consumer Control"));
        assert!(is_bad_name("OL action injector"));
        assert!(is_bad_name("Keyboard passthrough"));
        assert!(is_bad_name("Mouse passthrough (absolute)"));
        assert!(is_bad_name("FIFINE Microphones FIFINE K658 Microphone Consumer Control"));
        assert!(is_bad_name("HDA NVidia HDMI/DP,pcm=3"));
        assert!(is_bad_name("Power Button"));
        assert!(is_bad_name("Sleep Button"));
        assert!(is_bad_name("Video Bus"));
        assert!(is_bad_name("PC Speaker"));
        assert!(is_bad_name("Cranked Audio Virtual"));
        assert!(is_bad_name("HDA Intel PCH ALC1220 Headphone"));
        assert!(is_bad_name("ALC897 Front Mic"));

        // Good devices
        assert!(!is_bad_name("BY Tech Gaming Keyboard Consumer Control"));
        assert!(!is_bad_name("BY Tech Gaming Keyboard"));
        assert!(!is_bad_name("Logitech USB Receiver Consumer Control"));
        assert!(!is_bad_name("Keychron Q1 Pro"));
        assert!(!is_bad_name("Griffin PowerMate"));
        assert!(!is_bad_name("Surface Dial"));
        assert!(!is_bad_name("Corsair Falcon Keyboard"));
        assert!(!is_bad_name("Valcom Knob"));
    }

    #[test]
    fn midi_controls() {
        let control = |kind: &str, number, button, relative| MidiControl {
            channel: 1,
            kind: kind.into(),
            number,
            knob: 2,
            button,
            relative,
        };
        let controls = [
            control("cc", 0, false, false),
            control("cc", 32, true, false),
            control("note", 36, true, false),
            control("cc", 16, false, true),
        ];
        assert_eq!(midi_input(&controls, &[0xB0, 0, 127]), Some(Input::Set { knob: 1, percent: 100 }));
        assert_eq!(midi_input(&controls, &[0xB0, 0, 64]), Some(Input::Set { knob: 1, percent: 50 }));
        assert_eq!(midi_input(&controls, &[0xB1, 0, 64]), None, "other channel");
        assert_eq!(midi_input(&controls, &[0xB0, 32, 127]), Some(Input::Button { knob: 1, down: true }));
        assert_eq!(midi_input(&controls, &[0xB0, 32, 0]), Some(Input::Button { knob: 1, down: false }));
        assert_eq!(midi_input(&controls, &[0x90, 36, 100]), Some(Input::Button { knob: 1, down: true }));
        assert_eq!(midi_input(&controls, &[0x80, 36, 0]), Some(Input::Button { knob: 1, down: false }));
        assert_eq!(midi_input(&controls, &[0xB0, 16, 3]), Some(Input::Step { knob: 1, delta: 3 }));
        assert_eq!(midi_input(&controls, &[0xB0, 16, 126]), Some(Input::Step { knob: 1, delta: -2 }));
        assert_eq!(midi_message(&[0xF8]), None, "clock ticks are ignored");
    }

    #[test]
    fn deej_lines() {
        assert_eq!(deej_line("0|512|1023\r\n", false), [Some(0), Some(50), Some(100)]);
        assert_eq!(deej_line("0|1023", true), [Some(100), Some(0)]);
        assert_eq!(deej_line("12|x|2000", false), [Some(1), None, None]);
    }

    #[test]
    fn saved_midi_names_leave_out_the_alsa_numbers() {
        assert_eq!(stable_port_name("nanoKONTROL2:nanoKONTROL2 _ CTRL 20:0"), "nanoKONTROL2:nanoKONTROL2 _ CTRL");
        assert_eq!(stable_port_name("Cranked test:Cranked Test MIDI 128:0"), "Cranked test:Cranked Test MIDI");
        assert_eq!(stable_port_name("Launchkey MK3"), "Launchkey MK3");
        assert_eq!(stable_port_name("Port 2"), "Port 2");
    }

    #[test]
    fn deej_ports_are_boards_only() {
        assert!(is_board("2341") && is_board("1a86\n") && is_board("2e8a"));
        assert!(!is_board("043e"), "an LG monitor's control port isn't a slider box");
    }
}
