//! KDE global shortcuts: every knob's turn up / turn down / button, next profile and open the window, registered with
//! Plasma's shortcut service (kglobalaccel). They show up in System Settings > Shortcuts > Cranked Audio with no keys
//! set; whatever keys the user picks there work everywhere. Without kglobalaccel (other desktops) this does nothing.
//!
//! A shortcut's button acts like the mixer's: tap it for the press action, hold it for the hold action. Holding a
//! turn shortcut keeps turning (the key repeats).

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use crate::shortcuts::{all, Shortcut};
use crate::Event;

const COMPONENT: &str = "cranked-audio";
const COMPONENT_PATH: &str = "/component/cranked_audio";
const FRIENDLY: &str = "Cranked Audio";
/// kglobalaccel's "this action exists now" flag: loads the keys the user saved for it.
const SET_PRESENT: u32 = 2;

/// This desktop's shortcut service: KDE's kglobalaccel when it runs, else the XDG `GlobalShortcuts` portal (Hyprland and
/// others). `CRANKED_SHORTCUTS=portal` (or `kde`) picks one, for testing.
pub fn native() -> Box<dyn crate::shortcuts::GlobalShortcuts> {
    let kde_running = || {
        zbus::blocking::Connection::session()
            .and_then(|bus| {
                bus.call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "NameHasOwner",
                    &("org.kde.kglobalaccel",),
                )
            })
            .and_then(|reply| reply.body().deserialize::<bool>())
            .unwrap_or(false)
    };
    match std::env::var("CRANKED_SHORTCUTS").as_deref() {
        Ok("portal") => Box::new(super::shortcuts_portal::Portal::default()),
        Ok("kde") => Box::new(KGlobalAccel::default()),
        _ if kde_running() => Box::new(KGlobalAccel::default()),
        _ => Box::new(super::shortcuts_portal::Portal::default()),
    }
}

/// The Linux shortcuts adapter: KDE's kglobalaccel (the keys are picked in System Settings, not in our settings).
#[derive(Default)]
pub struct KGlobalAccel {
    running: Option<Shortcuts>,
}

impl crate::shortcuts::GlobalShortcuts for KGlobalAccel {
    fn editor(&self) -> cranked_ipc::ShortcutEditor {
        cranked_ipc::ShortcutEditor::Desktop
    }

    /// System Settings' shortcuts page.
    fn configure(&mut self) {
        crate::service::spawn_detached(std::path::Path::new("kcmshell6"), &["kcm_keys".as_ref()]);
    }

    fn start(&mut self, events: Sender<Event>, knobs: usize, _bindings: &BTreeMap<String, String>) {
        self.running = Shortcuts::start(events, knobs);
    }

    fn ensure(&mut self, knobs: usize, _bindings: &BTreeMap<String, String>) {
        if let Some(running) = &mut self.running {
            running.ensure(knobs);
        }
    }
}

struct Shortcuts {
    connection: zbus::blocking::Connection,
    registered: usize,
}

impl Shortcuts {
    /// Registers the shortcuts for `knobs` knobs and listens for them. None when there's no kglobalaccel.
    fn start(events: Sender<Event>, knobs: usize) -> Option<Shortcuts> {
        let connection = zbus::blocking::Connection::session().ok()?;
        let mut shortcuts = Shortcuts { connection, registered: 0 };
        if let Err(e) = shortcuts.register(0, knobs) {
            eprintln!("shortcuts: not available ({e})");
            return None;
        }
        shortcuts.registered = knobs;
        let connection = shortcuts.connection.clone();
        crate::platform::spawn_thread("shortcuts", move || listen(&connection, &events));
        Some(shortcuts)
    }

    /// Makes sure there are shortcuts for `knobs` knobs (ones for knobs since removed stay, keeping their keys).
    fn ensure(&mut self, knobs: usize) {
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
        .map(zbus::match_rule::Builder::build);
    let Ok(rule) = rule else { return };
    let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, connection, None) else {
        eprintln!("shortcuts: can't listen for them");
        return;
    };
    for message in messages.flatten() {
        let header = message.header();
        let down = match header.member().map(zbus::names::MemberName::as_str) {
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
