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

// ---------------------------------------------------------------- keyboard knobs

/// Input devices that have volume keys (candidates for a keyboard knob), by name.
pub fn keyboard_knobs() -> Vec<String> {
    let mut names: Vec<String> = evdev::enumerate()
        .filter(|(_, d)| d.supported_keys().is_some_and(|k| k.contains(evdev::KeyCode::KEY_VOLUMEUP)))
        .filter_map(|(_, d)| d.name().map(str::to_string))
        .filter(|n| !n.contains("Cranked Audio"))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// What a key does on a keyboard knob (`value`: 1 down, 2 repeat, 0 up). Media keys only matter when the input is
/// taken over (otherwise the desktop already handles them).
pub fn keyboard_input(code: evdev::KeyCode, value: i32, knob: usize, step: u8, grabbed: bool) -> Option<Input> {
    use evdev::KeyCode as K;
    let step = i32::from(step.max(1));
    match (code, value) {
        (K::KEY_VOLUMEUP, 1 | 2) => Some(Input::Step { knob, delta: step }),
        (K::KEY_VOLUMEDOWN, 1 | 2) => Some(Input::Step { knob, delta: -step }),
        (K::KEY_MUTE, 1) => Some(Input::Button { knob, down: true }),
        (K::KEY_MUTE, 0) => Some(Input::Button { knob, down: false }),
        (K::KEY_PLAYPAUSE, 1) if grabbed => Some(Input::Media(MediaKey::PlayPause)),
        (K::KEY_NEXTSONG, 1) if grabbed => Some(Input::Media(MediaKey::Next)),
        (K::KEY_PREVIOUSSONG, 1) if grabbed => Some(Input::Media(MediaKey::Previous)),
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

fn keyboard(name: &str, knob: usize, step: u8, grab: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    use std::os::fd::AsRawFd;
    let knob = knob.saturating_sub(1);
    let mut reported = false;
    while current() {
        let found = evdev::enumerate().find(|(_, d)| d.name() == Some(name));
        let Some((path, mut device)) = found else {
            if !reported {
                eprintln!("keyboard knob: {name:?} not found; waiting for it");
                reported = true;
            }
            std::thread::sleep(RETRY);
            continue;
        };
        let grabbed = grab && device.grab().is_ok();
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
                if let evdev::EventSummary::Key(_, code, value) = event.destructure() {
                    if let Some(input) = keyboard_input(code, value, knob, step, grabbed) {
                        if !send(events, input) {
                            return;
                        }
                    }
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
        if port_name.contains("Cranked Audio") {
            continue;
        }
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
pub fn serial_ports() -> Vec<String> {
    let mut ports: Vec<String> = std::fs::read_dir("/dev/serial/by-id")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
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
        assert_eq!(keyboard_input(K::KEY_PLAYPAUSE, 1, 3, 2, true), Some(Input::Media(MediaKey::PlayPause)));
        assert_eq!(keyboard_input(K::KEY_PLAYPAUSE, 1, 3, 2, false), None, "not taken over: the desktop handles it");
        assert_eq!(keyboard_input(K::KEY_A, 1, 3, 2, true), None);
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
}
