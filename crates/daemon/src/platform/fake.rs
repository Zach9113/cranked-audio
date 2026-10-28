//! Test adapters: every port records what the service asked of it, so the service can be tested without `PipeWire`,
//! a desktop session or a Windows VM.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};

use cranked_ipc::{AudioOutput, Snapshot};

use super::Platform;
use crate::audio::{self, AudioSystem};
use crate::dbus::RemoteApi;
use crate::desktop::{self, Desktop};
use crate::devices::{InputHardware, InputSource};
use crate::shortcuts::GlobalShortcuts;
use crate::tray::{Tray, TrayIcon};
use crate::updater::{ReleaseInfo, Updater};
use crate::Event;

/// What the service asked a fake adapter to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    Audio(audio::Command),
    Desktop(desktop::Command),
    Tray { profile: String, profiles: Vec<String>, mixer: bool },
}

/// A platform of fakes, and what they record in the order it happened.
pub fn platform() -> (Platform, Receiver<Recorded>) {
    let (record, recorded) = mpsc::channel();
    let platform = Platform {
        audio: Box::new(FakeAudio { record: record.clone() }),
        desktop: Box::new(FakeDesktop { record: record.clone() }),
        tray: Box::new(FakeTray { record }),
        shortcuts: Box::new(NoShortcuts),
        remote_api: Box::new(NoRemoteApi),
        input_hardware: Box::new(NoInputHardware),
        updater: Box::new(NoUpdates),
    };
    (platform, recorded)
}

/// Passes each command the service sends on its channel to the recording, on a thread like a real adapter.
fn record_all<T>(commands: Receiver<T>, record: Sender<Recorded>, wrap: fn(T) -> Recorded)
where
    T: Send + 'static,
{
    crate::platform::spawn_thread("fake", move || {
        for command in commands {
            if record.send(wrap(command)).is_err() {
                return;
            }
        }
    });
}

struct FakeAudio {
    record: Sender<Recorded>,
}

impl AudioSystem for FakeAudio {
    fn start(&self, commands: Receiver<audio::Command>, _events: Sender<Event>) {
        record_all(commands, self.record.clone(), Recorded::Audio);
    }

    fn outputs(&self) -> Vec<AudioOutput> {
        Vec::new()
    }
}

struct FakeDesktop {
    record: Sender<Recorded>,
}

impl Desktop for FakeDesktop {
    fn start(&self, commands: Receiver<desktop::Command>) {
        record_all(commands, self.record.clone(), Recorded::Desktop);
    }

    fn watch_screen(&self, _events: Sender<Event>) {}
}

struct FakeTray {
    record: Sender<Recorded>,
}

impl TrayIcon for FakeTray {
    fn start(&mut self, _tray: Tray) {}

    fn show(&self, profile: String, profiles: Vec<String>, mixer: bool) {
        let _ = self.record.send(Recorded::Tray { profile, profiles, mixer });
    }
}

struct NoShortcuts;

impl GlobalShortcuts for NoShortcuts {
    fn start(&mut self, _events: Sender<Event>, _knobs: usize, _bindings: &BTreeMap<String, String>) {}

    fn ensure(&mut self, _knobs: usize, _bindings: &BTreeMap<String, String>) {}
}

struct NoRemoteApi;

impl RemoteApi for NoRemoteApi {
    fn start(&mut self, _events: Sender<Event>) {}

    fn publish(&self, _snapshot: &Snapshot) {}
}

struct NoInputHardware;

impl InputHardware for NoInputHardware {
    fn keyboard_knobs(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }

    fn serial_ports(&self) -> Vec<String> {
        Vec::new()
    }

    fn keyboard_knob(&self, _name: String, _knob: usize, _step: u8, _grab: bool) -> Box<dyn InputSource> {
        Box::new(NoInput)
    }
}

struct NoInput;

impl InputSource for NoInput {
    fn run(&self, _events: &Sender<Event>, _current: &dyn Fn() -> bool) {}
}

struct NoUpdates;

impl Updater for NoUpdates {
    fn check(&self, events: Sender<Event>) {
        let _ = events.send(Event::UpdateCheckResult(Ok(None)));
    }

    fn can_install(&self) -> bool {
        false
    }

    fn install(&self, _release: ReleaseInfo, _events: Sender<Event>) {}

    fn restarts_service(&self) -> bool {
        false
    }

    fn after_restart(&self, _old_pid: Option<u32>) {}
}
