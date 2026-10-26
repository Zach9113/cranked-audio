//! Windows: serial ports by USB vendor; no keyboard knobs yet (Windows can't hand one keyboard's keys to a single
//! program the way evdev's grab does).

use std::sync::mpsc::Sender;

use super::is_board;
use crate::Event;

pub fn keyboard_knobs() -> (Vec<String>, Vec<String>) {
    (Vec::new(), Vec::new())
}

pub fn keyboard(
    name: &str,
    _knob: usize,
    _step: u8,
    _grab: bool,
    _events: &Sender<Event>,
    _current: &dyn Fn() -> bool,
) {
    eprintln!("keyboard knob: {name:?} isn't supported on Windows yet");
}

/// Serial ports that could be a deej box: USB boards only, and not the mixer.
pub fn serial_ports() -> Vec<String> {
    let Ok(ports) = serialport::available_ports() else { return Vec::new() };
    let mut ports: Vec<String> = ports
        .into_iter()
        .filter(|port| match &port.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                is_board(&format!("{:04x}", info.vid)) && !crate::serial::is_mixer(info.vid, info.pid)
            }
            _ => false,
        })
        .map(|port| port.port_name)
        .collect();
    ports.sort();
    ports
}
