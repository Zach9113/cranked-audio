//! Linux: the tray item over D-Bus (`StatusNotifierItem`, through ksni).

use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};

use crate::tray::{Action, Tray, ICONS};

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
                data: rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect(),
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
                    options: self
                        .profiles
                        .iter()
                        .map(|p| RadioItem { label: p.clone(), ..Default::default() })
                        .collect(),
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

/// The Linux tray adapter: a `StatusNotifierItem` over D-Bus (through ksni), which Plasma and most panels show.
#[derive(Default)]
pub struct StatusNotifier {
    running: Option<Handle<Tray>>,
}

impl crate::tray::TrayIcon for StatusNotifier {
    fn start(&mut self, tray: Tray) {
        self.running = start(tray);
    }

    fn show(&self, profile: String, profiles: Vec<String>, mixer: bool) {
        if let Some(running) = &self.running {
            running.update(move |tray| {
                (tray.profile, tray.profiles, tray.mixer) = (profile, profiles, mixer);
            });
        }
    }
}

fn start(tray: Tray) -> Option<Handle<Tray>> {
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tray: not available ({e})");
            None
        }
    }
}
