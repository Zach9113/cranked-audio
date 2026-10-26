//! Turning settings into the choices the pages show, and back.

use cranked_ipc::settings::Action;
use cranked_ipc::Entity;

pub const ACTIONS: [&str; 9] = [
    "Nothing",
    "Mute this knob",
    "Play / Pause",
    "Next track",
    "Previous track",
    "Switch audio output",
    "Next profile",
    "Home Assistant: toggle",
    "Home Assistant: turn off",
];

pub fn action_index(action: &Action) -> usize {
    match action {
        Action::None => 0,
        Action::Mute => 1,
        Action::PlayPause => 2,
        Action::Next => 3,
        Action::Previous => 4,
        Action::SwitchOutput => 5,
        Action::NextProfile => 6,
        Action::HomeAssistantToggle(_) => 7,
        Action::HomeAssistantOff(_) => 8,
    }
}

pub fn action_entity(action: &Action) -> Option<&str> {
    match action {
        Action::HomeAssistantToggle(entity) | Action::HomeAssistantOff(entity) => Some(entity),
        _ => None,
    }
}

/// A Home Assistant action without an entity picked means nothing.
pub fn action_from(index: usize, entity: Option<String>) -> Action {
    match (index, entity) {
        (1, _) => Action::Mute,
        (2, _) => Action::PlayPause,
        (3, _) => Action::Next,
        (4, _) => Action::Previous,
        (5, _) => Action::SwitchOutput,
        (6, _) => Action::NextProfile,
        (7, Some(entity)) => Action::HomeAssistantToggle(entity),
        (8, Some(entity)) => Action::HomeAssistantOff(entity),
        _ => Action::None,
    }
}

pub const AMOUNTS: [u8; 4] = [20, 30, 50, 70];

pub fn amount_index(amount: u8) -> usize {
    AMOUNTS.iter().enumerate().min_by_key(|(_, a)| a.abs_diff(amount)).map_or(1, |(i, _)| i)
}

pub const SWATCHES: [&str; 14] = [
    "#FF3B30", "#FF7A1A", "#FFC21A", "#B8E62E", "#22C55E", "#14B8A6", "#06B6D4", "#3B82F6", "#6366F1", "#A855F7",
    "#EC4899", "#FFFFFF", "#404048", "#000000",
];

/// A picker's (labels, ids): a "none" entry first, then the matching entities by name. Ids in `keep` that Home
/// Assistant didn't list (not loaded yet, or gone) are kept so saving never drops them.
pub fn choices(entities: &[Entity], domains: &[&str], none: &str, keep: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut labels = vec![none.to_string()];
    let mut ids = vec![String::new()];
    let wanted = |id: &str| domains.is_empty() || domains.contains(&id.split('.').next().unwrap_or_default());
    for entity in entities.iter().filter(|e| wanted(&e.id)) {
        labels.push(entity.name.clone());
        ids.push(entity.id.clone());
    }
    for id in keep {
        if !id.is_empty() && !ids.iter().any(|known| known == id) {
            labels.push((*id).to_string());
            ids.push((*id).to_string());
        }
    }
    (labels, ids)
}

pub fn index_of(ids: &[String], id: Option<&str>) -> usize {
    id.and_then(|id| ids.iter().position(|known| known == id)).unwrap_or(0)
}

/// One row of the app picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppChoice {
    pub name: String,
    /// On this knob.
    pub selected: bool,
    pub playing: bool,
    /// Other knobs that have this app ("Music", "Music, Game").
    pub elsewhere: String,
}

/// Every app worth offering for this knob: what it has, what's been seen playing, and what other knobs have.
/// Each app once, spelled the way this knob spells it. This knob's apps come first, then what's playing, then apps
/// on no knob, then apps on other knobs; alphabetical (case-insensitive) within each group.
pub fn app_choices(
    mine: &[String],
    known: &[String],
    playing: &[String],
    others: &[(String, Vec<String>)],
) -> Vec<AppChoice> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut names: Vec<String> = Vec::new();
    let all = mine.iter().chain(known).chain(playing).chain(others.iter().flat_map(|(_, apps)| apps));
    for name in all.map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if !names.iter().any(|n| same(n, name)) {
            names.push(name.to_string());
        }
    }
    let mut rows: Vec<AppChoice> = names
        .into_iter()
        .map(|name| AppChoice {
            selected: mine.iter().any(|m| same(m, &name)),
            playing: playing.iter().any(|p| same(p, &name)),
            elsewhere: others
                .iter()
                .filter(|(_, apps)| apps.iter().any(|a| same(a, &name)))
                .map(|(knob, _)| knob.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            name,
        })
        .collect();
    let group = |r: &AppChoice| match (r.selected, r.playing, r.elsewhere.is_empty()) {
        (true, _, _) => 0,
        (false, true, _) => 1,
        (false, false, true) => 2,
        _ => 3,
    };
    rows.sort_by_key(|r| (group(r), r.name.to_lowercase()));
    rows
}

/// Adds a knob to a profile (up to the most a profile can have), with its own colour. Returns its index.
pub fn add_knob(profile: &mut cranked_ipc::settings::Profile) -> Option<usize> {
    let count = profile.channels.len();
    if count >= cranked_ipc::settings::MAX_CONTROLS {
        return None;
    }
    // bright swatches only, in order, so new knobs are easy to tell apart
    let colours = &SWATCHES[..11];
    profile.channels.push(cranked_ipc::settings::Channel {
        name: format!("Knob {}", count + 1),
        color: colours[count % colours.len()].into(),
        background: "#000000".into(),
        ..Default::default()
    });
    Some(count)
}

/// Removes knob `index` from a profile, keeping knob numbers elsewhere (Auto-lower, hidden knobs) pointing at the
/// same knobs. A profile always keeps at least one knob.
pub fn remove_knob(config: &mut cranked_ipc::settings::Config, profile: usize, index: usize) -> bool {
    let Some(p) = config.profiles.get_mut(profile) else { return false };
    if p.channels.len() <= 1 || index >= p.channels.len() {
        return false;
    }
    p.channels.remove(index);
    let knob_num = index + 1;
    // knob numbers in settings count from 1
    let renumber = |list: &mut Vec<usize>| {
        list.retain(|&n| n != knob_num);
        for n in list.iter_mut() {
            if *n > knob_num {
                *n -= 1;
            }
        }
    };
    renumber(&mut p.auto_lower.lower);
    renumber(&mut p.auto_lower.triggers);
    renumber(&mut config.look.hidden_knobs);
    config.devices.retain_mut(|d| match d {
        cranked_ipc::settings::Device::KeyboardKnob { knob, name: _, step: _, grab: _ } => {
            if *knob == knob_num {
                false
            } else {
                if *knob > knob_num {
                    *knob -= 1;
                }
                true
            }
        }
        cranked_ipc::settings::Device::Midi { controls, name: _ } => {
            controls.retain_mut(|c| {
                if c.knob == knob_num {
                    false
                } else {
                    if c.knob > knob_num {
                        c.knob -= 1;
                    }
                    true
                }
            });
            !controls.is_empty()
        }
        cranked_ipc::settings::Device::Deej { knobs, port: _, baud: _, invert: _ } => {
            knobs.retain_mut(|k| {
                if *k == knob_num {
                    false
                } else {
                    if *k > knob_num {
                        *k -= 1;
                    }
                    true
                }
            });
            !knobs.is_empty()
        }
    });
    true
}

/// One line describing a configured device, for Settings > Devices. `knob` names knobs (counting from 1).
pub fn device_summary(device: &cranked_ipc::settings::Device, knob: &dyn Fn(usize) -> String) -> String {
    use cranked_ipc::settings::Device;
    match device {
        Device::KeyboardKnob { name, knob: k, step, grab } => format!(
            "Knob controller \u{2014} {name}: turns {} by {step}%{}",
            knob(*k),
            if *grab { "" } else { " (the system volume moves too)" }
        ),
        Device::Midi { name, controls } => {
            let parts: Vec<String> = controls
                .iter()
                .map(|c| {
                    let what = if c.button {
                        "button"
                    } else if c.relative {
                        "endless knob"
                    } else {
                        "knob"
                    };
                    format!("{} {} ch{} \u{2192} {}'s {what}", c.kind.to_uppercase(), c.number, c.channel, knob(c.knob))
                })
                .collect();
            let list = if parts.is_empty() { "nothing mapped yet".to_string() } else { parts.join(", ") };
            format!("MIDI \u{2014} {name}: {list}")
        }
        Device::Deej { port, knobs, invert, baud: _ } => {
            let list: Vec<String> = knobs.iter().map(|k| knob(*k)).collect();
            format!(
                "deej box \u{2014} {port}: sliders turn {}{}",
                list.join(", "),
                if *invert { " (reversed)" } else { "" }
            )
        }
    }
}

/// Adds a learned MIDI control to the device for its port (making the device if it's new). A control that was
/// already mapped is replaced, so learning the same fader again just moves it.
pub fn add_midi_control(
    devices: &mut Vec<cranked_ipc::settings::Device>,
    port: &str,
    control: cranked_ipc::settings::MidiControl,
) {
    use cranked_ipc::settings::Device;
    let same = |c: &cranked_ipc::settings::MidiControl| {
        c.channel == control.channel && c.kind == control.kind && c.number == control.number
    };
    for device in devices.iter_mut() {
        if let Device::Midi { name, controls } = device {
            if name == port {
                controls.retain(|c| !same(c));
                controls.push(control);
                return;
            }
        }
    }
    devices.push(Device::Midi { name: port.to_string(), controls: vec![control] });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_round_trip() {
        for action in [
            Action::None,
            Action::Mute,
            Action::PlayPause,
            Action::Next,
            Action::Previous,
            Action::SwitchOutput,
            Action::NextProfile,
            Action::HomeAssistantToggle("switch.coffee_maker".into()),
            Action::HomeAssistantOff("light.desk_lamp".into()),
        ] {
            let back = action_from(action_index(&action), action_entity(&action).map(String::from));
            assert_eq!(back, action);
        }
        assert_eq!(action_from(7, None), Action::None);
    }

    #[test]
    fn pickers_keep_unknown_entities() {
        let entities = [
            Entity { id: "light.desk_lamp".into(), name: "Desk Lamp".into() },
            Entity { id: "switch.coffee_maker".into(), name: "Coffee Maker".into() },
        ];
        let (labels, ids) = choices(&entities, &["light"], "(none)", &["light.desk_lamp", "light.gone"]);
        assert_eq!(labels, ["(none)", "Desk Lamp", "light.gone"]);
        assert_eq!(index_of(&ids, Some("light.gone")), 2);
        assert_eq!(index_of(&ids, None), 0);
        assert_eq!(amount_index(30), 1);
        assert_eq!(amount_index(45), 2);
    }

    #[test]
    fn app_list_merges_everything_once_and_marks_this_knobs_apps() {
        let s = |v: &[&str]| v.iter().map(std::string::ToString::to_string).collect::<Vec<_>>();
        let rows = app_choices(
            &s(&["Firefox", "mpv"]),
            &s(&["firefox", "Spotify", "vesktop"]),
            &s(&["vesktop"]),
            &[("Music".into(), s(&["Spotify"])), ("Game".into(), s(&["Balatro", "spotify"]))],
        );
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        // this knob's, then playing, then on no knob, then on other knobs
        assert_eq!(names, ["Firefox", "mpv", "vesktop", "Balatro", "Spotify"]);
        assert!(rows[0].selected && rows[1].selected && !rows[2].selected);
        assert!(rows[2].playing && !rows[0].playing);
        assert_eq!(rows[4].elsewhere, "Music, Game");
    }

    #[test]
    fn knobs_can_be_added_and_removed() {
        use cranked_ipc::settings::Config;
        let mut config = Config::default();
        let start = config.profiles[0].channels.len();
        assert_eq!(add_knob(&mut config.profiles[0]), Some(start));
        assert_eq!(config.profiles[0].channels[start].name, format!("Knob {}", start + 1));
        config.profiles[0].auto_lower.lower = vec![1, 3, 6];
        config.look.hidden_knobs = vec![2, 6];
        config.devices.push(cranked_ipc::settings::Device::KeyboardKnob {
            name: "TestKeyboard".into(),
            knob: 6,
            step: 2,
            grab: true,
        });
        assert!(remove_knob(&mut config, 0, 2), "remove knob 3");
        assert_eq!(config.profiles[0].auto_lower.lower, [1, 5], "knob 3 gone, knob 6 is now knob 5");
        assert_eq!(config.look.hidden_knobs, [2, 5]);
        assert_eq!(
            config.devices,
            vec![cranked_ipc::settings::Device::KeyboardKnob {
                name: "TestKeyboard".into(),
                knob: 5,
                step: 2,
                grab: true,
            }]
        );
        while config.profiles[0].channels.len() > 1 {
            assert!(remove_knob(&mut config, 0, 0));
        }
        assert!(!remove_knob(&mut config, 0, 0), "the last knob stays");
        for _ in 0..20 {
            add_knob(&mut config.profiles[0]);
        }
        assert_eq!(config.profiles[0].channels.len(), cranked_ipc::settings::MAX_CONTROLS);
    }

    #[test]
    fn learning_a_midi_control_twice_moves_it() {
        use cranked_ipc::settings::{Device, MidiControl};
        let control =
            |number, knob| MidiControl { channel: 1, kind: "cc".into(), number, knob, button: false, relative: false };
        let mut devices = Vec::new();
        add_midi_control(&mut devices, "nanoKONTROL2", control(0, 1));
        add_midi_control(&mut devices, "nanoKONTROL2", control(1, 2));
        add_midi_control(&mut devices, "nanoKONTROL2", control(0, 3));
        add_midi_control(&mut devices, "Other", control(0, 1));
        assert_eq!(devices.len(), 2);
        let Device::Midi { controls, name: _ } = &devices[0] else { panic!() };
        assert_eq!(controls.iter().map(|c| (c.number, c.knob)).collect::<Vec<_>>(), [(1, 2), (0, 3)]);
        let name = |k: usize| format!("K{k}");
        assert_eq!(device_summary(&devices[1], &name), "MIDI \u{2014} Other: CC 0 ch1 \u{2192} K1's knob");
        let deej = Device::Deej { port: "/dev/ttyACM0".into(), baud: 9600, knobs: vec![1, 2], invert: true };
        assert_eq!(device_summary(&deej, &name), "deej box \u{2014} /dev/ttyACM0: sliders turn K1, K2 (reversed)");
    }
}
