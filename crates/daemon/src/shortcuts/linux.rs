//! KDE global shortcuts: every knob's turn up / turn down / button, next profile and open the window, registered with
//! Plasma's shortcut service (kglobalaccel). They show up in System Settings > Shortcuts > Cranked Audio with no keys
//! set; whatever keys the user picks there work everywhere. Without kglobalaccel (other desktops) this does nothing.
//!
//! A shortcut's button acts like the mixer's: tap it for the press action, hold it for the hold action. Holding a
//! turn shortcut keeps turning (the key repeats).

use std::sync::mpsc::Sender;

use crate::dbus::Request;
use crate::Event;

const COMPONENT: &str = "cranked-audio";
const COMPONENT_PATH: &str = "/component/cranked_audio";
const FRIENDLY: &str = "Cranked Audio";
/// Points one turn shortcut (or one key repeat) moves a knob.
const STEP: i32 = 2;
/// kglobalaccel's "this action exists now" flag: loads the keys the user saved for it.
const SET_PRESENT: u32 = 2;

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
    pub fn id(self) -> String {
        match self {
            Shortcut::Up(k) => format!("knob-{}-up", k + 1),
            Shortcut::Down(k) => format!("knob-{}-down", k + 1),
            Shortcut::Button(k) => format!("knob-{}-button", k + 1),
            Shortcut::NextProfile => "next-profile".into(),
            Shortcut::OpenWindow => "open-window".into(),
        }
    }

    fn friendly(self) -> String {
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
pub fn all(knobs: usize) -> Vec<Shortcut> {
    let mut list = Vec::new();
    for knob in 0..knobs {
        list.extend([Shortcut::Up(knob), Shortcut::Down(knob), Shortcut::Button(knob)]);
    }
    list.extend([Shortcut::NextProfile, Shortcut::OpenWindow]);
    list
}

pub struct Shortcuts {
    connection: zbus::blocking::Connection,
    registered: usize,
}

impl Shortcuts {
    /// Registers the shortcuts for `knobs` knobs and listens for them. None when there's no kglobalaccel.
    pub fn start(events: Sender<Event>, knobs: usize) -> Option<Shortcuts> {
        let connection = zbus::blocking::Connection::session().ok()?;
        let mut shortcuts = Shortcuts { connection, registered: 0 };
        if let Err(e) = shortcuts.register(0, knobs) {
            eprintln!("shortcuts: not available ({e})");
            return None;
        }
        shortcuts.registered = knobs;
        let connection = shortcuts.connection.clone();
        std::thread::spawn(move || listen(&connection, &events));
        Some(shortcuts)
    }

    /// Makes sure there are shortcuts for `knobs` knobs (ones for knobs since removed stay, keeping their keys).
    pub fn ensure(&mut self, knobs: usize) {
        if knobs > self.registered {
            match self.register(self.registered, knobs) {
                Ok(()) => self.registered = knobs,
                Err(e) => eprintln!("shortcuts: {e}"),
            }
        }
    }

    fn register(&self, from: usize, knobs: usize) -> zbus::Result<()> {
        let proxy = zbus::blocking::Proxy::new(
            &self.connection,
            "org.kde.kglobalaccel",
            "/kglobalaccel",
            "org.kde.KGlobalAccel",
        )?;
        for shortcut in all(knobs) {
            if matches!(shortcut, Shortcut::Up(k) | Shortcut::Down(k) | Shortcut::Button(k) if k < from) {
                continue;
            }
            if from > 0 && matches!(shortcut, Shortcut::NextProfile | Shortcut::OpenWindow) {
                continue;
            }
            let action = vec![COMPONENT.to_string(), shortcut.id(), FRIENDLY.to_string(), shortcut.friendly()];
            proxy.call::<_, _, ()>("doRegister", &(action.clone(),))?;
            let no_keys: Vec<(Vec<i32>,)> = Vec::new();
            proxy.call::<_, _, Vec<(Vec<i32>,)>>("setShortcutKeys", &(action, no_keys, SET_PRESENT))?;
        }
        Ok(())
    }
}

fn listen(connection: &zbus::blocking::Connection, events: &Sender<Event>) {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.kde.kglobalaccel.Component")
        .and_then(|r| r.path(COMPONENT_PATH))
        .map(|r| r.build());
    let Ok(rule) = rule else { return };
    let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, connection, None) else {
        eprintln!("shortcuts: can't listen for them");
        return;
    };
    for message in messages.flatten() {
        let header = message.header();
        let down = match header.member().map(|m| m.as_str()) {
            Some("globalShortcutPressed") => Some(true),
            Some("globalShortcutReleased") => Some(false),
            Some("globalShortcutRepeated") => None,
            _ => continue,
        };
        let Ok((component, action, _)) = message.body().deserialize::<(String, String, i64)>() else { continue };
        if component != COMPONENT {
            continue;
        }
        if let Some(request) = Shortcut::from_id(&action).and_then(|s| s.request(down)) {
            if events.send(Event::Remote(request)).is_err() {
                return;
            }
        }
    }
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
