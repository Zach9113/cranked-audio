//! Windows: no session bus, so no API (the window and tray still work).

use std::sync::mpsc::Sender;

use crate::Event;

pub fn start(_events: Sender<Event>) -> Option<((), ())> {
    None
}

pub fn publish(_connection: &(), _state: &(), _json: String) {}
