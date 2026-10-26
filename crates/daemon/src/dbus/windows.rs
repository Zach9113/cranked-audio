//! Windows: no session bus, so no API (the window and tray still work).

use std::sync::mpsc::Sender;

use crate::Event;

/// Nothing to hold: there is no API.
pub type Published = ((), ());

pub fn start(_events: Sender<Event>) -> Option<Published> {
    None
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "the same signature as the Linux version")]
pub fn publish(_connection: &(), _state: &(), _json: &str) {}
