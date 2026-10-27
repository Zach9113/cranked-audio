//! Global shortcuts for the knobs, buttons, next profile and the window: KDE's `kglobalaccel` on Linux, and
//! `RegisterHotKey` on Windows (configured in `config.toml` under `[shortcuts]` or in Settings).

#[cfg(not(windows))]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use crate::dbus::Request;
use crate::Event;

/// The port to the desktop's global shortcuts. Each platform has one adapter (`linux::KGlobalAccel`,
/// `windows::HotKeys`), chosen in `platform::native()`; it keeps its own registrations.
pub trait GlobalShortcuts {
    /// Registers the shortcuts for `knobs` knobs (keys from `bindings` where the platform takes them from the
    /// settings) and reports presses as `Event::Remote`. Without a shortcut service it says so once and stays away.
    fn start(&mut self, events: Sender<Event>, knobs: usize, bindings: &BTreeMap<String, String>);
    /// Catches up with new settings: shortcuts for more knobs, or changed keys.
    fn ensure(&mut self, knobs: usize, bindings: &BTreeMap<String, String>);
}

/// Points one turn shortcut (or one key repeat) moves a knob.
pub const STEP: i32 = 2;

/// What a shortcut action does when its keys go down, repeat or come up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Up(usize),
    Down(usize),
    Button(usize),
    NextProfile,
    OpenWindow,
}

impl Shortcut {
    #[cfg_attr(
        all(windows, not(test)),
        expect(dead_code, reason = "Windows keys shortcuts by the settings, not by id")
    )]
    pub fn id(self) -> String {
        match self {
            Shortcut::Up(k) => format!("knob-{}-up", k + 1),
            Shortcut::Down(k) => format!("knob-{}-down", k + 1),
            Shortcut::Button(k) => format!("knob-{}-button", k + 1),
            Shortcut::NextProfile => "next-profile".into(),
            Shortcut::OpenWindow => "open-window".into(),
        }
    }

    #[cfg_attr(windows, expect(dead_code))]
    pub fn friendly(self) -> String {
        match self {
            Shortcut::Up(k) => format!("Knob {}: turn up", k + 1),
            Shortcut::Down(k) => format!("Knob {}: turn down", k + 1),
            Shortcut::Button(k) => format!("Knob {}: button (tap = press, hold = hold)", k + 1),
            Shortcut::NextProfile => "Next profile".into(),
            Shortcut::OpenWindow => "Open Cranked Audio".into(),
        }
    }

    pub fn from_id(id: &str) -> Option<Shortcut> {
        match id {
            "next-profile" => return Some(Shortcut::NextProfile),
            "open-window" => return Some(Shortcut::OpenWindow),
            _ => {}
        }
        let rest = id.strip_prefix("knob-")?;
        let (number, what) = rest.split_once('-')?;
        let knob = number.parse::<usize>().ok()?.checked_sub(1)?;
        match what {
            "up" => Some(Shortcut::Up(knob)),
            "down" => Some(Shortcut::Down(knob)),
            "button" => Some(Shortcut::Button(knob)),
            _ => None,
        }
    }

    /// What the keys going down (`Some(true)`), repeating (`None`) or coming up (`Some(false)`) asks for.
    pub fn request(self, down: Option<bool>) -> Option<Request> {
        match (self, down) {
            (Shortcut::Up(knob), Some(true) | None) => Some(Request::StepKnob { knob, delta: STEP }),
            (Shortcut::Down(knob), Some(true) | None) => Some(Request::StepKnob { knob, delta: -STEP }),
            (Shortcut::Button(button), Some(down)) => Some(Request::Button { button, down }),
            (Shortcut::NextProfile, Some(true)) => Some(Request::NextProfile),
            (Shortcut::OpenWindow, Some(true)) => Some(Request::OpenWindow),
            _ => None,
        }
    }
}

/// The shortcuts for `knobs` knobs.
#[cfg_attr(all(windows, not(test)), expect(dead_code, reason = "Windows registers only shortcuts that have keys"))]
pub fn all(knobs: usize) -> Vec<Shortcut> {
    let mut list = Vec::new();
    for knob in 0..knobs {
        list.extend([Shortcut::Up(knob), Shortcut::Down(knob), Shortcut::Button(knob)]);
    }
    list.extend([Shortcut::NextProfile, Shortcut::OpenWindow]);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_read_back_and_buttons_follow_the_keys() {
        for shortcut in all(16) {
            assert_eq!(Shortcut::from_id(&shortcut.id()), Some(shortcut));
        }
        assert_eq!(all(5).len(), 17);
        assert_eq!(Shortcut::from_id("knob-0-up"), None);
        let button = Shortcut::Button(2);
        assert_eq!(button.request(Some(true)), Some(Request::Button { button: 2, down: true }));
        assert_eq!(button.request(Some(false)), Some(Request::Button { button: 2, down: false }));
        assert_eq!(button.request(None), None, "a held key's repeats don't press again");
        assert_eq!(
            Shortcut::Up(0).request(None),
            Some(Request::StepKnob { knob: 0, delta: STEP }),
            "held: keeps turning"
        );
        assert_eq!(Shortcut::Up(0).request(Some(false)), None);
    }
}
