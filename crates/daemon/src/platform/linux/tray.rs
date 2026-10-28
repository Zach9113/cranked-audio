//! Linux: the tray item over D-Bus (`StatusNotifierItem`, through ksni).

use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};

use crate::config::Action as ControlAction;
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
        let item = |label: &str, icon: &str, action: Action| -> MenuItem<Self> {
            StandardItem {
                label: label.into(),
                icon_name: icon.into(),
                activate: Box::new(move |tray: &mut Self| tray.send(action.clone())),
                ..Default::default()
            }
            .into()
        };
        let mut items: Vec<MenuItem<Self>> = vec![
            item("Open Cranked Audio", "", Action::OpenWindow),
            item("Settings…", "configure", Action::OpenSettings),
            MenuItem::Separator,
            item("Play/Pause", "media-playback-start", Action::Run(ControlAction::PlayPause)),
            item("Next track", "media-skip-forward", Action::Run(ControlAction::Next)),
            item("Previous track", "media-skip-backward", Action::Run(ControlAction::Previous)),
            item("Switch output", "audio-card", Action::Run(ControlAction::SwitchOutput)),
        ];
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
        items.push(item("Edit settings file…", "document-edit", Action::EditSettings));
        items.push(item("Quit", "application-exit", Action::Quit));
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
