//! `cranked-audio --waybar [KNOB]`: the state as Waybar custom-module JSON, one line per change, for panels outside
//! KDE (Hyprland, Sway, niri). With a knob number (from 1) it shows that knob; without, a summary of them all.

use cranked_ipc::{Knob, Snapshot};
use serde_json::json;

/// What the module shows for `snapshot`: one knob (0-based), or all of them when `knob` is None.
pub fn module(snapshot: &Snapshot, knob: Option<usize>) -> serde_json::Value {
    match knob {
        Some(index) => match snapshot.knobs.get(index) {
            Some(knob) => one(knob, index),
            None => json!({ "text": "", "tooltip": format!("Cranked Audio has no knob {}", index + 1) }),
        },
        None => all(snapshot),
    }
}

fn one(knob: &Knob, index: usize) -> serde_json::Value {
    let number = index + 1;
    let tooltip = format!(
        "{}: {}\n{}\nScroll to turn · click: {} · right-click: open Cranked Audio",
        knob.name,
        state(knob),
        knob.source,
        knob.press.to_lowercase()
    );
    let mut value = json!({
        "text": format!("{} {}", knob.name, level(knob)),
        "alt": format!("knob-{number}"),
        "tooltip": tooltip,
        "class": classes(knob),
    });
    if let Some(level) = knob.level {
        value["percentage"] = json!(level);
    }
    value
}

fn all(snapshot: &Snapshot) -> serde_json::Value {
    let text = snapshot.knobs.iter().map(|k| format!("{} {}", k.name, level(k))).collect::<Vec<_>>().join(" · ");
    let lines = snapshot.knobs.iter().map(|k| format!("{}: {} ({})", k.name, state(k), k.source));
    let tooltip = std::iter::once(format!("Cranked Audio · {}", snapshot.profile)).chain(lines).collect::<Vec<_>>();
    json!({ "text": text, "alt": snapshot.profile, "tooltip": tooltip.join("\n"), "class": "all" })
}

/// `65%`, or `–` before the knob has a level.
fn level(knob: &Knob) -> String {
    knob.level.map_or_else(|| "–".to_owned(), |level| format!("{level}%"))
}

fn state(knob: &Knob) -> String {
    let mut text = level(knob);
    if knob.muted {
        text.push_str(", muted");
    }
    if knob.lowered {
        text.push_str(", lowered");
    }
    text
}

/// CSS classes for styling: `muted`, `lowered`, and `idle` (nothing playing on the knob).
fn classes(knob: &Knob) -> Vec<&'static str> {
    let mut classes = Vec::new();
    if knob.muted {
        classes.push("muted");
    }
    if knob.lowered {
        classes.push("lowered");
    }
    if knob.level.is_none() || knob.source == "Nothing playing" {
        classes.push("idle");
    }
    classes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knob(name: &str, level: Option<u8>, muted: bool) -> Knob {
        Knob {
            name: name.into(),
            level,
            min: 0,
            max: 100,
            color: "#FF7A1A".into(),
            background: "#2A1200".into(),
            muted_color: "#000000".into(),
            muted,
            lowered: false,
            source: "Firefox".into(),
            press: "Play/Pause".into(),
            hold: "Mute".into(),
            held: false,
        }
    }

    #[test]
    fn one_knob_shows_its_level_and_state() {
        let snapshot =
            Snapshot { knobs: vec![knob("Media", Some(65), false), knob("Chat", None, true)], ..Snapshot::default() };
        let media = module(&snapshot, Some(0));
        assert_eq!(media["text"], "Media 65%");
        assert_eq!(media["percentage"], 65);
        assert_eq!(media["class"], json!([]));
        assert!(media["tooltip"].as_str().is_some_and(|t| t.contains("click: play/pause")));
        let chat = module(&snapshot, Some(1));
        assert_eq!(chat["text"], "Chat –");
        assert_eq!(chat["class"], json!(["muted", "idle"]));
        assert!(chat.get("percentage").is_none(), "no level, no percentage");
        assert_eq!(module(&snapshot, Some(5))["text"], "", "a knob that isn't there shows nothing");
    }

    #[test]
    fn all_knobs_make_one_line() {
        let snapshot = Snapshot {
            profile: "Main".into(),
            knobs: vec![knob("Media", Some(65), false), knob("Music", Some(40), false)],
            ..Snapshot::default()
        };
        let summary = module(&snapshot, None);
        assert_eq!(summary["text"], "Media 65% · Music 40%");
        assert_eq!(summary["tooltip"], "Cranked Audio · Main\nMedia: 65% (Firefox)\nMusic: 40% (Firefox)");
    }
}
