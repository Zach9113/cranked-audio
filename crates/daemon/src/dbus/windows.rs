//! Windows: no session bus, so no API (the window and tray still work).

use std::sync::mpsc::Sender;

use crate::Event;

/// Nothing to hold: there is no API.
pub type Published = ((), ());

pub fn start(_events: Sender<Event>) -> Option<Published> {
    None
}

pub fn publish(_connection: &(), _state: &(), _json: &str) {}
