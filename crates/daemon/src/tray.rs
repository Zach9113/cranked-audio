//! Tray icon: open the window, switch profiles, quit. Lives in the service so it's there without a window.

use std::sync::mpsc::Sender;

use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};
use ksni::blocking::{Handle, TrayMethods};

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
    (22, include_bytes!("../../../data/tray/22.rgba")),
    (32, include_bytes!("../../../data/tray/32.rgba")),
    (48, include_bytes!("../../../data/tray/48.rgba")),
    (64, include_bytes!("../../../data/tray/64.rgba")),
];

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "cranked-audio".into()
    }

    fn title(&self) -> String {
        "Cranked Audio".into()
    }

    /// The icon travels with the tray (not looked up by name), so it can't be mixed up with another program's.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        ICONS
            .iter()
            .map(|(size, rgba)| ksni::Icon {
                width: *size,
                height: *size,
                // RGBA -> the ARGB (network byte order) the tray spec wants
                data: rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect(),
            })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let mixer = if self.mixer { "Mixer connected" } else { "No mixer: on-screen controls" };
        ksni::ToolTip {
            title: "Cranked Audio".into(),
            description: format!("{mixer}\nProfile: {}", self.profile),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Action::OpenWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![StandardItem {
            label: "Open Cranked Audio".into(),
            activate: Box::new(|tray: &mut Self| tray.send(Action::OpenWindow)),
            ..Default::default()
        }
        .into()];
        if self.profiles.len() > 1 {
            items.push(MenuItem::Separator);
            items.push(
                RadioGroup {
                    selected: self.profiles.iter().position(|p| *p == self.profile).unwrap_or(0),
                    select: Box::new(|tray: &mut Self, index: usize| {
                        if let Some(name) = tray.profiles.get(index).cloned() {
                            tray.send(Action::Profile(name));
                        }
                    }),
                    options: self.profiles.iter().map(|p| RadioItem { label: p.clone(), ..Default::default() }).collect(),
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Edit settings file…".into(),
                icon_name: "document-edit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::EditSettings)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Action::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn start(tray: Tray) -> Option<Handle<Tray>> {
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tray: not available ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ICONS;

    #[test]
    fn tray_images_are_complete() {
        for (size, rgba) in ICONS {
            assert_eq!(rgba.len(), (size * size * 4) as usize);
        }
    }
}
