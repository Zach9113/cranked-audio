//! Windows: no session bus, so no API (the window and tray still work).

use std::sync::mpsc::Sender;

use crate::Event;

/// The Windows remote API adapter: there is none yet, so nothing is offered.
pub struct NotOffered;

impl crate::dbus::RemoteApi for NotOffered {
    fn start(&mut self, _events: Sender<Event>) {}

    fn publish(&self, _snapshot: &cranked_ipc::Snapshot) {}
}
