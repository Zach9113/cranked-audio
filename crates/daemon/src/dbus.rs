//! The session D-Bus API, for the Plasma panel widget, scripts, KDE Connect commands and automations:
//! `io.github.zach9113.CrankedAudio` at `/io/github/zach9113/CrankedAudio`, interface
//! `io.github.zach9113.CrankedAudio1`. State is the same JSON snapshot the window gets.
//!
//! ```text
//! busctl --user call io.github.zach9113.CrankedAudio /io/github/zach9113/CrankedAudio \
//!     io.github.zach9113.CrankedAudio1 SetKnob uu 1 40
//! ```

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Event;

pub const NAME: &str = "io.github.zach9113.CrankedAudio";
pub const PATH: &str = "/io/github/zach9113/CrankedAudio";
pub const INTERFACE: &str = "io.github.zach9113.CrankedAudio1";

/// What a D-Bus caller asked for; the main loop carries it out with the same rules as everything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    SetKnob { knob: usize, percent: u8 },
    StepKnob { knob: usize, delta: i32 },
    Button { button: usize, down: bool },
    Profile(String),
    OpenWindow,
    OpenSettings,
    NextProfile,
}

struct Api {
    events: Sender<Event>,
    /// The latest snapshot as JSON, kept current by the main loop.
    state: Arc<Mutex<String>>,
}

impl Api {
    fn send(&self, request: Request) {
        let _ = self.events.send(Event::Remote(request));
    }
}

#[zbus::interface(name = "io.github.zach9113.CrankedAudio1")]
impl Api {
    /// Everything the window shows, as JSON (knobs with names, levels, colours, what's playing, button actions).
    fn get_state(&self) -> String {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Turn knob `knob` (0-based) to `percent` (0..100), like turning it by hand.
    fn set_knob(&self, knob: u32, percent: u32) {
        self.send(Request::SetKnob { knob: knob as usize, percent: percent.min(100) as u8 });
    }

    /// Turn knob `knob` by `delta` points (negative turns it down).
    fn step_knob(&self, knob: u32, delta: i32) {
        self.send(Request::StepKnob { knob: knob as usize, delta });
    }

    /// Press button `button` (its press action).
    fn press(&self, button: u32) {
        self.send(Request::Button { button: button as usize, down: true });
        self.send(Request::Button { button: button as usize, down: false });
    }

    /// Hold button `button` (its hold action).
    fn hold(&self, button: u32) {
        let (events, button) = (self.events.clone(), button as usize);
        self.send(Request::Button { button, down: true });
        std::thread::spawn(move || {
            std::thread::sleep(crate::buttons::HOLD_AFTER + Duration::from_millis(100));
            let _ = events.send(Event::Remote(Request::Button { button, down: false }));
        });
    }

    /// Button `button` goes down / comes up, for callers that time presses themselves.
    fn button_down(&self, button: u32) {
        self.send(Request::Button { button: button as usize, down: true });
    }

    fn button_up(&self, button: u32) {
        self.send(Request::Button { button: button as usize, down: false });
    }

    fn set_profile(&self, name: &str) {
        self.send(Request::Profile(name.to_string()));
    }

    fn open_window(&self) {
        self.send(Request::OpenWindow);
    }

    /// Opens the window on its Settings page.
    fn open_settings(&self) {
        self.send(Request::OpenSettings);
    }

    /// Sent whenever the state changes, with the new JSON.
    #[zbus(signal)]
    async fn state_changed(emitter: &zbus::object_server::SignalEmitter<'_>, state: &str) -> zbus::Result<()>;
}

/// Publishes the API; returns the connection (for signals) and the shared state, or None if the bus refuses.
pub fn start(events: Sender<Event>) -> Option<(zbus::blocking::Connection, Arc<Mutex<String>>)> {
    let state = Arc::new(Mutex::new(String::new()));
    let api = Api { events, state: Arc::clone(&state) };
    // tests run their own copy under another name ($CRANKED_DBUS_NAME) next to a real one
    let name = std::env::var("CRANKED_DBUS_NAME").unwrap_or_else(|_| NAME.to_string());
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(name))
        .and_then(|b| b.serve_at(PATH, api))
        .and_then(|b| b.build());
    match connection {
        Ok(connection) => Some((connection, state)),
        Err(e) => {
            eprintln!("d-bus api: not available ({e})");
            None
        }
    }
}

/// Stores the new state and tells listeners.
pub fn publish(connection: &zbus::blocking::Connection, state: &Mutex<String>, json: String) {
    if let Ok(mut current) = state.lock() {
        if *current == json {
            return;
        }
        current.clone_from(&json);
    }
    let _ = connection.emit_signal(None::<&str>, PATH, INTERFACE, "StateChanged", &(json.as_str(),));
}
