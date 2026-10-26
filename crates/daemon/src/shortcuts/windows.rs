//! Windows: no global shortcuts yet (there's no system-wide place to pick their keys like KDE's).

use std::sync::mpsc::Sender;

use crate::Event;

pub struct Shortcuts;

impl Shortcuts {
    pub fn start(_events: Sender<Event>, _knobs: usize) -> Option<Shortcuts> {
        None
    }

    #[expect(clippy::unused_self, reason = "the same signature as the Linux version")]
    pub fn ensure(&mut self, _knobs: usize) {}
}
