//! The mixer's USB serial port (SPEC.md section 1).

use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use serialport::COMPort as Port;
use serialport::SerialPortType;
#[cfg(not(windows))]
use serialport::TTYPort as Port;

use crate::Event;

const BY_ID: &str = "/dev/serial/by-id";
const NAME_PREFIX: &str = "usb-1a86_USB_";
const WCH_VID: u16 = 0x1a86;
/// The mixer's chip (CH343). Windows matches it exactly: CH340 Arduino clones (deej boxes) share the vendor id.
#[cfg(windows)]
const MIXER_PID: u16 = 0x55d3;

#[cfg(windows)]
pub fn is_mixer(vid: u16, pid: u16) -> bool {
    vid == WCH_VID && pid == MIXER_PID
}

pub fn find_port() -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(BY_ID) {
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(NAME_PREFIX))
            .map(|entry| entry.path())
            .collect();
        found.sort();
        if let Some(port) = found.into_iter().next() {
            return Some(port);
        }
    }
    // fallback for environments without /dev/serial/by-id: match QinHeng (WCH) USB vendor ID
    if let Ok(ports) = serialport::available_ports() {
        for port in ports {
            if let SerialPortType::UsbPort(info) = port.port_type {
                #[cfg(windows)]
                let found = is_mixer(info.vid, info.pid);
                #[cfg(not(windows))]
                let found = info.vid == WCH_VID;
                if found {
                    return Some(PathBuf::from(port.port_name));
                }
            }
        }
    }
    None
}

/// How long a read waits for the mixer. On Windows the reader thread also sends the LED frames between reads, so
/// this is how late a frame can be.
#[cfg(windows)]
const READ_TIMEOUT: Duration = Duration::from_millis(10);
#[cfg(not(windows))]
const READ_TIMEOUT: Duration = Duration::from_millis(500);

fn open(path: &Path) -> serialport::Result<Port> {
    #[cfg_attr(windows, expect(unused_mut, reason = "only Linux changes the port after opening"))]
    let mut port = serialport::new(path.to_string_lossy(), 115_200)
        .timeout(READ_TIMEOUT)
        // raising DTR resets the board, which then reports every knob (Linux raises it on open by itself)
        .dtr_on_open(true)
        .open_native()?;
    // Windows opens COM ports exclusively anyway
    #[cfg(not(windows))]
    port.set_exclusive(true)?;
    Ok(port)
}

/// Shared handle for sending LED frames while the reader thread owns the port.
#[cfg(not(windows))]
#[derive(Clone, Default)]
pub struct Writer(Arc<Mutex<Option<Port>>>);

#[cfg(not(windows))]
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

    /// Frames are written straight away here.
    pub fn flush(&self) {}

    fn set(&self, port: Option<Port>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = port;
        }
    }
}

/// The newest LED frame, for the reader thread to send between reads. Windows serial handles are synchronous and a
/// cloned handle shares the reader's, so a write from another thread waits behind the pending read (frames came
/// hundreds of ms late and the board blanked its LEDs).
#[cfg(windows)]
#[derive(Clone, Default)]
pub struct Writer(Arc<Mutex<Option<Vec<u8>>>>);

#[cfg(windows)]
impl Writer {
    pub fn send(&self, bytes: &[u8]) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(bytes.to_vec());
        }
    }

    /// Gives the reader thread a moment to send the last frame (before quitting).
    pub fn flush(&self) {
        for _ in 0..10 {
            if self.0.lock().map_or(true, |slot| slot.is_none()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn take(&self) -> Option<Vec<u8>> {
        self.0.lock().ok()?.take()
    }
}

/// Runs forever on its own thread: waits for the mixer, forwards its bytes, starts over when it goes away.
pub fn run(events: &Sender<Event>, writer: &Writer) {
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
                if matches!(e.kind(), serialport::ErrorKind::Io(ErrorKind::PermissionDenied)) {
                    eprintln!(
                        "mixer: permission denied on {}. Add your user to group 'uucp' (Arch) or 'dialout' (Ubuntu/Debian/Fedora)",
                        path.display()
                    );
                } else {
                    // most likely another program has it; don't keep resetting the board by retrying fast
                    eprintln!("mixer: can't open {}: {e}; trying again in 30 s", path.display());
                }
                std::thread::sleep(Duration::from_secs(30));
                continue;
            }
        };
        eprintln!("mixer: connected ({})", path.display());
        #[cfg(not(windows))]
        writer.set(port.try_clone_native().map_err(|e| eprintln!("mixer: no LED handle ({e})")).ok());
        #[cfg(windows)]
        writer.take(); // a frame from before the connect is stale
        if events.send(Event::MixerConnected).is_err() {
            return;
        }
        let mut buffer = [0_u8; 256];
        loop {
            match port.read(&mut buffer) {
                Ok(0) if cfg!(not(windows)) && !path.exists() => {
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
            #[cfg(windows)]
            if let Some(frame) = writer.take() {
                if let Err(e) = port.write_all(&frame) {
                    eprintln!("mixer: LED write failed ({e})");
                }
            }
        }
        #[cfg(not(windows))]
        writer.set(None);
        let _ = events.send(Event::MixerLost);
        std::thread::sleep(Duration::from_millis(500));
    }
}
