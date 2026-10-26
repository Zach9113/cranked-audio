//! Tray icon: open the window, switch profiles, quit. Lives in the service so it's there without a window.

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::start;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::start;

use std::sync::mpsc::Sender;

use crate::Event;

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
