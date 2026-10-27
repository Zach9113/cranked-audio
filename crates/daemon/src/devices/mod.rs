//! Controllers besides the mixer: keyboard knobs (evdev), MIDI controllers and deej-style Arduino slider boxes.
//! Each runs on its own thread and turns what it reads into the same knob and button input as everything else.
//! Threads stop once `generation` moves past theirs (new settings started replacements).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cranked_ipc::settings::{Device, MidiControl};
use cranked_protocol::convert::saturate_u8;

#[cfg(not(windows))]
pub mod linux;
#[cfg(windows)]
pub mod windows;

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

/// The port to one configured controller. It runs on its own thread and turns what it reads into `Input` until
/// `current()` turns false (new settings started its replacement).
pub trait InputSource: Send {
    fn run(&self, events: &Sender<Event>, current: &dyn Fn() -> bool);
}

/// The port to this platform's input hardware: keyboard knobs read through the OS, and the serial ports to offer.
/// Each platform has one adapter (`linux::Evdev`, `windows::RawInput`), chosen in `platform::native()`.
pub trait InputHardware {
    /// Keyboard knobs to offer in Settings, and the keyboards that only type (offered further down).
    fn keyboard_knobs(&self) -> (Vec<String>, Vec<String>);
    /// Serial ports to offer for deej boxes.
    fn serial_ports(&self) -> Vec<String>;
    /// A configured keyboard knob as an input source.
    fn keyboard_knob(&self, name: String, knob: usize, step: u8, grab: bool) -> Box<dyn InputSource>;
}

/// The configured devices as input sources: keyboard knobs from this platform's hardware, MIDI controllers and deej
/// boxes from the portable code below.
pub fn sources(devices: &[Device], hardware: &dyn InputHardware) -> Vec<Box<dyn InputSource>> {
    devices
        .iter()
        .cloned()
        .map(|device| -> Box<dyn InputSource> {
            match device {
                Device::KeyboardKnob { name, knob, step, grab } => hardware.keyboard_knob(name, knob, step, grab),
                Device::Midi { name, controls } => Box::new(MidiController { name, controls }),
                Device::Deej { port, baud, knobs, invert } => Box::new(DeejBox { port, baud, knobs, invert }),
            }
        })
        .collect()
}

/// Starts every source on its own thread; the ones started before stop once they notice the newer generation.
pub fn start(sources: Vec<Box<dyn InputSource>>, events: &Sender<Event>, generation: &Arc<AtomicU64>) {
    let mine = generation.fetch_add(1, Ordering::Relaxed) + 1;
    for source in sources {
        let (events, generation) = (events.clone(), Arc::clone(generation));
        let current = move || generation.load(Ordering::Relaxed) == mine;
        crate::platform::spawn_thread("device", move || source.run(&events, &current));
    }
}

fn send(events: &Sender<Event>, input: Input) -> bool {
    events.send(Event::Device(input)).is_ok()
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
    Some(Input::Set { knob, percent: saturate_u8((u32::from(value) * 100 + 63) / 127) })
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

/// A MIDI controller (portable through midir).
struct MidiController {
    name: String,
    controls: Vec<MidiControl>,
}

impl InputSource for MidiController {
    fn run(&self, events: &Sender<Event>, current: &dyn Fn() -> bool) {
        midi(&self.name, &self.controls, events, current);
    }
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
            move |_, message, ()| {
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
pub fn learn_midi(events: &Sender<Event>, wait: Duration) {
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
            move |_, message, ()| {
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
                let percent = saturate_u8((raw * 100 + 511) / 1023);
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
/// FTDI, `CP210x`, PL2303), Adafruit, `SparkFun`, Teensy, Raspberry Pi (Pico) and Espressif (ESP32).
const BOARD_VENDORS: [&str; 11] =
    ["2341", "2a03", "1a86", "0403", "10c4", "067b", "239a", "1b4f", "16c0", "2e8a", "303a"];

pub fn is_board(vendor: &str) -> bool {
    BOARD_VENDORS.contains(&vendor.trim())
}

/// A deej-style slider box on a serial port (portable through serialport).
struct DeejBox {
    port: String,
    baud: u32,
    knobs: Vec<usize>,
    invert: bool,
}

impl InputSource for DeejBox {
    fn run(&self, events: &Sender<Event>, current: &dyn Fn() -> bool) {
        deej(&self.port, self.baud, &self.knobs, self.invert, events, current);
    }
}

fn deej(port: &str, baud: u32, knobs: &[usize], invert: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    use std::io::{BufRead as _, BufReader};
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

    /// Stands in for a platform's keyboard knob: reports one turn up, then stops.
    struct FakeKnob {
        knob: usize,
    }

    impl InputSource for FakeKnob {
        fn run(&self, events: &Sender<Event>, _current: &dyn Fn() -> bool) {
            send(events, Input::Step { knob: self.knob, delta: 1 });
        }
    }

    struct FakeHardware;

    impl InputHardware for FakeHardware {
        fn keyboard_knobs(&self) -> (Vec<String>, Vec<String>) {
            (Vec::new(), Vec::new())
        }

        fn serial_ports(&self) -> Vec<String> {
            Vec::new()
        }

        fn keyboard_knob(&self, _name: String, knob: usize, _step: u8, _grab: bool) -> Box<dyn InputSource> {
            Box::new(FakeKnob { knob })
        }
    }

    #[test]
    fn configured_devices_run_as_sources_without_real_hardware() {
        let keyboard_knob = Device::KeyboardKnob { name: "Fake knob".into(), knob: 2, step: 2, grab: true };
        let deej_box = Device::Deej { port: "/dev/null".into(), baud: 9600, knobs: vec![0], invert: false };
        assert_eq!(sources(&[keyboard_knob.clone(), deej_box], &FakeHardware).len(), 2);

        let (events, inbox) = std::sync::mpsc::channel();
        start(sources(&[keyboard_knob], &FakeHardware), &events, &Arc::new(AtomicU64::new(0)));
        let reported = inbox.recv_timeout(Duration::from_secs(2));
        assert!(
            matches!(reported, Ok(Event::Device(Input::Step { knob: 2, delta: 1 }))),
            "the fake knob's turn arrives"
        );
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
