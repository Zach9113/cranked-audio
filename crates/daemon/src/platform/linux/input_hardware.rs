//! Linux: keyboard knobs and knob controllers through evdev, and serial ports by their /sys USB vendor.

use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::desktop::MediaKey;
use crate::devices::{is_board, send, Input, RETRY};
use crate::Event;

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
    let lower = name.to_lowercase();
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
fn keyboard_knobs() -> (Vec<String>, Vec<String>) {
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
fn readable(fd: std::os::fd::BorrowedFd, timeout: Duration) -> bool {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
    let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
    let timeout = PollTimeout::try_from(timeout).unwrap_or(PollTimeout::MAX);
    poll(&mut fds, timeout).is_ok_and(|ready| ready > 0)
        && fds[0].revents().is_some_and(|events| events.contains(PollFlags::POLLIN))
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

/// The Linux input hardware adapter: keyboard knobs through evdev, serial ports from /dev/serial/by-id.
pub struct Evdev;

impl crate::devices::InputHardware for Evdev {
    fn keyboard_knobs(&self) -> (Vec<String>, Vec<String>) {
        keyboard_knobs()
    }

    fn serial_ports(&self) -> Vec<String> {
        serial_ports()
    }

    fn keyboard_knob(&self, name: String, knob: usize, step: u8, grab: bool) -> Box<dyn crate::devices::InputSource> {
        Box::new(EvdevKnob { name, knob, step, grab })
    }
}

/// A keyboard knob read through evdev (with `grab`, the system volume doesn't move too).
struct EvdevKnob {
    name: String,
    knob: usize,
    step: u8,
    grab: bool,
}

impl crate::devices::InputSource for EvdevKnob {
    fn run(&self, events: &Sender<Event>, current: &dyn Fn() -> bool) {
        keyboard(&self.name, self.knob, self.step, self.grab, events, current);
    }
}

fn keyboard(name: &str, knob: usize, step: u8, grab: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    use std::os::fd::AsFd as _;
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
        'read: while current() {
            if !readable(device.as_fd(), Duration::from_secs(1)) {
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

/// The USB vendor id of a serial device (a /dev path or a /dev/serial/by-id link).
fn usb_vendor(path: &std::path::Path) -> Option<String> {
    let tty = std::fs::canonicalize(path).ok()?;
    let name = tty.file_name()?.to_str()?;
    std::fs::read_to_string(format!("/sys/class/tty/{name}/device/../idVendor")).ok()
}

/// Serial ports that could be a deej box: USB boards only (not, say, a monitor's control port), and not the mixer.
fn serial_ports() -> Vec<String> {
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
}
