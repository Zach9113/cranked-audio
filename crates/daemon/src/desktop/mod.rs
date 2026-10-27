//! The desktop around the app: media keys for the active player, volume popups, and whether the screen is off.

use std::sync::mpsc::{Receiver, Sender};

use cranked_ipc::settings::PopupStyle;

use crate::Event;

/// The port to the desktop around the app. Each platform has one adapter (`linux::Plasma`, `windows::Win32`),
/// chosen in `platform::native()`.
pub trait Desktop {
    /// Starts the desktop thread: it presses media keys for the active player and shows volume and text popups.
    fn start(&self, commands: Receiver<Command>);
    /// Starts the screen watcher: it reports `Event::ScreenOff(true)` while the screen is locked or every monitor
    /// sleeps, and `false` once it's back.
    fn watch_screen(&self, events: Sender<Event>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Media(MediaKey),
    /// Volume popup with a name ("Music 40%").
    Volume {
        percent: u8,
        name: String,
        style: PopupStyle,
    },
    Text {
        icon: String,
        text: String,
        style: PopupStyle,
    },
}
