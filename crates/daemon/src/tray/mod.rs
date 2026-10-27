//! Tray icon: open the window, switch profiles, quit. Lives in the service so it's there without a window.

#[cfg(not(windows))]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use std::sync::mpsc::Sender;

use crate::Event;

/// The port to the tray icon. Each platform has one adapter (`linux::StatusNotifier`, `windows::NotificationArea`),
/// chosen in `platform::native()`; it keeps its own running icon.
pub trait TrayIcon {
    /// Puts the icon up. Without a tray on this desktop it says so once and stays away.
    fn start(&mut self, tray: Tray);
    /// Shows the current profile, the profiles to pick from and whether the mixer is connected.
    fn show(&self, profile: String, profiles: Vec<String>, mixer: bool);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenWindow,
    Profile(String),
    EditSettings,
    Quit,
}

pub struct Tray {
    pub events: Sender<Event>,
    pub profile: String,
    pub profiles: Vec<String>,
    pub mixer: bool,
}

impl Tray {
    fn send(&self, action: Action) {
        let _ = self.events.send(Event::Tray(action));
    }
}

/// Pre-rendered by data/make-tray-icons.sh.
const ICONS: [(i32, &[u8]); 4] = [
    (22, include_bytes!("../../../../data/tray/22.rgba")),
    (32, include_bytes!("../../../../data/tray/32.rgba")),
    (48, include_bytes!("../../../../data/tray/48.rgba")),
    (64, include_bytes!("../../../../data/tray/64.rgba")),
];

#[cfg(test)]
mod tests {
    use super::ICONS;

    #[test]
    fn tray_images_are_complete() {
        for (size, rgba) in ICONS {
            assert_eq!(rgba.len(), usize::try_from(size * size * 4).unwrap());
        }
    }
}
