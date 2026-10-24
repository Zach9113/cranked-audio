//! The mixer's USB serial port (SPEC.md section 1).

use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serialport::TTYPort;

use crate::Event;

const BY_ID: &str = "/dev/serial/by-id";
const NAME_PREFIX: &str = "usb-1a86_USB_Single_Serial_";

pub fn find_port() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(BY_ID)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(NAME_PREFIX))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found.into_iter().next()
}

fn open(path: &PathBuf) -> serialport::Result<TTYPort> {
    let mut port = serialport::new(path.to_string_lossy(), 115_200).timeout(Duration::from_millis(500)).open_native()?;
    port.set_exclusive(true)?;
    Ok(port)
}

/// Shared handle for sending LED frames while the reader thread owns the port.
#[derive(Clone, Default)]
pub struct Writer(Arc<Mutex<Option<TTYPort>>>);

impl Writer {
    pub fn send(&self, bytes: &[u8]) {
        if let Ok(mut port) = self.0.lock() {
            if let Some(port) = port.as_mut() {
                if let Err(e) = port.write_all(bytes) {
                    eprintln!("mixer: LED write failed ({e})");
                }
            }
        }
    }

    fn set(&self, port: Option<TTYPort>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = port;
        }
    }
}

/// Runs forever on its own thread: waits for the mixer, forwards its bytes, starts over when it goes away.
pub fn run(events: Sender<Event>, writer: Writer) {
    let mut announced_missing = false;
    loop {
        let Some(path) = find_port() else {
            if !announced_missing {
                eprintln!("mixer: not connected, waiting");
                announced_missing = true;
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        announced_missing = false;
        let mut port = match open(&path) {
            Ok(port) => port,
            Err(e) => {
                // most likely another program has it; don't keep resetting the board by retrying fast
                eprintln!("mixer: can't open {}: {e}; trying again in 30 s", path.display());
                std::thread::sleep(Duration::from_secs(30));
                continue;
            }
        };
        eprintln!("mixer: connected ({})", path.display());
        writer.set(port.try_clone_native().map_err(|e| eprintln!("mixer: no LED handle ({e})")).ok());
        if events.send(Event::MixerConnected).is_err() {
            return;
        }
        let mut buffer = [0u8; 256];
        loop {
            match port.read(&mut buffer) {
                Ok(0) if !path.exists() => {
                    eprintln!("mixer: unplugged");
                    break;
                }
                Ok(0) => {}
                Ok(n) => {
                    if events.send(Event::Bytes(buffer[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    eprintln!("mixer: lost ({e})");
                    break;
                }
            }
        }
        writer.set(None);
        let _ = events.send(Event::MixerLost);
        std::thread::sleep(Duration::from_millis(500));
    }
}
