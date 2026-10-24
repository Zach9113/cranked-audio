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
        Action::HaToggle(_) => 7,
        Action::HaOff(_) => 8,
    }
}

pub fn action_entity(action: &Action) -> Option<&str> {
    match action {
        Action::HaToggle(entity) | Action::HaOff(entity) => Some(entity),
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
        (7, Some(entity)) => Action::HaToggle(entity),
        (8, Some(entity)) => Action::HaOff(entity),
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
            Action::HaToggle("switch.coffee_maker".into()),
            Action::HaOff("light.desk_lamp".into()),
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
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
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
}
