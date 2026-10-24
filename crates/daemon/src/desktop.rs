//! Session D-Bus: media players (MPRIS) and KDE's on-screen popups.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

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
    },
    Text {
        icon: String,
        text: String,
    },
}

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// playerctld tracks the most recently active player for us when it's running.
const PLAYERCTLD: &str = "org.mpris.MediaPlayer2.playerctld";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

fn list_names(bus: &Connection) -> zbus::Result<Vec<String>> {
    bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListNames",
        &(),
    )?
    .body()
    .deserialize()
}

fn playback_status(bus: &Connection, name: &str) -> Option<String> {
    let reply = bus
        .call_method(
            Some(name),
            PLAYER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(PLAYER_IFACE, "PlaybackStatus"),
        )
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    String::try_from(value).ok()
}

/// Playing beats paused beats anything else; playerctld wins when it's there.
pub fn pick_player(names: &[String], status: impl Fn(&str) -> Option<String>) -> Option<String> {
    let players: Vec<&String> = names.iter().filter(|n| n.starts_with(MPRIS_PREFIX)).collect();
    if let Some(forced) = std::env::var_os("CRANKED_MPRIS_PLAYER") {
        let forced = forced.to_string_lossy();
        return players.into_iter().find(|n| **n == forced).cloned();
    }
    if players.iter().any(|n| *n == PLAYERCTLD) {
        return Some(PLAYERCTLD.to_string());
    }
    let rank = |name: &str| match status(name).as_deref() {
        Some("Playing") => 0,
        Some("Paused") => 1,
        _ => 2,
    };
    players.into_iter().min_by_key(|n| rank(n)).cloned()
}

fn media(bus: &Connection, key: MediaKey) -> zbus::Result<()> {
    let names = list_names(bus)?;
    let Some(player) = pick_player(&names, |name| playback_status(bus, name)) else {
        eprintln!("media: no player");
        return Ok(());
    };
    let method = match key {
        MediaKey::PlayPause => "PlayPause",
        MediaKey::Next => "Next",
        MediaKey::Previous => "Previous",
    };
    bus.call_method(Some(player.as_str()), PLAYER_PATH, Some(PLAYER_IFACE), method, &())?;
    Ok(())
}

fn osd<B>(bus: &Connection, method: &str, body: &B) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    bus.call_method(Some("org.kde.plasmashell"), "/org/kde/osdService", Some("org.kde.osdService"), method, body)?;
    Ok(())
}

/// Fallback for non-KDE environments (GNOME, Sway, Hyprland, XFCE): standard freedesktop notifications.
fn notify(bus: &Connection, icon: &str, summary: &str, body: &str, value: Option<i32>) -> zbus::Result<()> {
    let mut hints = HashMap::<&str, Value>::new();
    hints.insert("transient", Value::from(true));
    if let Some(val) = value {
        hints.insert("value", Value::from(val));
        hints.insert("x-canonical-private-synchronous", Value::from("volume"));
    }
    let actions: &[&str] = &[];
    bus.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("Cranked Audio", 0u32, icon, summary, body, actions, hints, 1000i32),
    )?;
    Ok(())
}

fn run_one(bus: &Connection, command: Command) -> zbus::Result<()> {
    match command {
        Command::Media(key) => media(bus, key),
        Command::Volume { percent, name } => {
            if osd(bus, "mediaPlayerVolumeChanged", &(i32::from(percent), name.as_str(), "audio-volume-high")).is_err()
            {
                notify(bus, "audio-volume-high", &format!("{name} {percent}%"), "", Some(i32::from(percent)))?;
            }
            Ok(())
        }
        Command::Text { icon, text } => {
            if osd(bus, "showText", &(icon.as_str(), text.as_str())).is_err() {
                notify(bus, &icon, &text, "", None)?;
            }
            Ok(())
        }
    }
}

/// Runs forever on its own thread.
pub fn run(commands: Receiver<Command>) {
    let mut bus: Option<Connection> = None;
    let mut warned_popup = false;
    for command in commands {
        if bus.is_none() {
            bus = Connection::session().map_err(|e| eprintln!("desktop: no session bus ({e})")).ok();
        }
        let Some(connection) = &bus else { continue };
        let is_popup = !matches!(command, Command::Media(_));
        if let Err(e) = run_one(connection, command) {
            if !is_popup {
                eprintln!("media: {e}");
            } else if !warned_popup {
                eprintln!("popups: {e} (not on KDE Plasma?)");
                warned_popup = true;
            }
        }
    }
}

/// How often the screen watcher looks (two tiny reads: one D-Bus call, one file per monitor).
const SCREEN_EVERY: Duration = Duration::from_secs(2);

/// Every connected monitor is powered down (`dpms` lines from /sys/class/drm/*/), or there are none.
pub fn monitors_asleep(connectors: &[(String, String)]) -> bool {
    let connected: Vec<&String> =
        connectors.iter().filter(|(status, _)| status == "connected").map(|(_, dpms)| dpms).collect();
    !connected.is_empty() && connected.iter().all(|dpms| dpms.as_str() != "On")
}

fn read_connectors() -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let dir = e.path();
            let status = std::fs::read_to_string(dir.join("status")).ok()?;
            let dpms = std::fs::read_to_string(dir.join("dpms")).ok()?;
            Some((status.trim().to_string(), dpms.trim().to_string()))
        })
        .collect()
}

fn screen_locked(bus: &Connection) -> Option<bool> {
    let reply = bus
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "GetActive",
            &(),
        )
        .ok()?;
    reply.body().deserialize().ok()
}

/// Reports `Event::ScreenOff(true)` while the screen is locked or all monitors sleep, `false` when it's back.
pub fn watch_screen(events: Sender<crate::Event>) {
    let bus = Connection::session().ok();
    let mut last = None;
    loop {
        let locked = bus.as_ref().and_then(screen_locked).unwrap_or(false);
        let off = locked || monitors_asleep(&read_connectors());
        if last != Some(off) {
            last = Some(off);
            if events.send(crate::Event::ScreenOff(off)).is_err() {
                return;
            }
        }
        std::thread::sleep(SCREEN_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn playerctld_knows_best() {
        let all = names(&["org.kde.kwin", "org.mpris.MediaPlayer2.spotify", PLAYERCTLD]);
        assert_eq!(pick_player(&all, |_| None).as_deref(), Some(PLAYERCTLD));
    }

    #[test]
    fn otherwise_the_playing_player() {
        let all = names(&["org.mpris.MediaPlayer2.firefox", "org.mpris.MediaPlayer2.spotify", "org.kde.kwin"]);
        let status = |n: &str| Some(if n.ends_with("spotify") { "Playing" } else { "Paused" }.to_string());
        assert_eq!(pick_player(&all, status).as_deref(), Some("org.mpris.MediaPlayer2.spotify"));
        assert_eq!(pick_player(&names(&["org.kde.kwin"]), |_| None), None);
    }

    #[test]
    fn monitors_asleep_only_when_every_connected_one_is_off() {
        let c = |status: &str, dpms: &str| (status.to_string(), dpms.to_string());
        assert!(!monitors_asleep(&[c("connected", "On"), c("connected", "Off")]));
        assert!(monitors_asleep(&[c("connected", "Off"), c("connected", "Standby"), c("disconnected", "On")]));
        assert!(!monitors_asleep(&[c("disconnected", "Off")]), "no monitors known: don't guess");
    }
}
