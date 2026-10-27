//! The desktop around the app: media keys for the active player, volume popups, and whether the screen is off.

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::{run, watch_screen};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{attach_input_desktop, run, watch_screen, CRANKED_EXTRA_INFO};

use cranked_ipc::settings::PopupStyle;

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
