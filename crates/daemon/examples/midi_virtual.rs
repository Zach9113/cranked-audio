//! Test helper for tests/live_devices.sh: publishes a virtual MIDI controller called "Cranked Test MIDI" and sends
//! each stdin line as one MIDI message, written as hex bytes ("b0 00 7f").

#[cfg(unix)]
use midir::os::unix::VirtualOutput;

#[cfg(not(unix))]
fn main() {
    eprintln!("virtual MIDI ports need ALSA or CoreMIDI; Windows has none");
}

#[cfg(unix)]
fn main() {
    use std::io::BufRead;

    let output = midir::MidiOutput::new("Cranked test").expect("MIDI");
    let mut port = output.create_virtual("Cranked Test MIDI").expect("virtual MIDI port");
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let bytes: Vec<u8> = line.split_whitespace().filter_map(|b| u8::from_str_radix(b, 16).ok()).collect();
        if !bytes.is_empty() {
            let _ = port.send(&bytes);
        }
    }
}
