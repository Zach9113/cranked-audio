//! Session D-Bus: media players (MPRIS) and KDE's on-screen popups.

use std::sync::mpsc::Receiver;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

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
    Volume { percent: u8, name: String },
    Text { icon: String, text: String },
}

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// playerctld tracks the most recently active player for us when it's running.
const PLAYERCTLD: &str = "org.mpris.MediaPlayer2.playerctld";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

fn list_names(bus: &Connection) -> zbus::Result<Vec<String>> {
    bus.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())?
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

fn run_one(bus: &Connection, command: Command) -> zbus::Result<()> {
    match command {
        Command::Media(key) => media(bus, key),
        Command::Volume { percent, name } => {
            osd(bus, "mediaPlayerVolumeChanged", &(i32::from(percent), name.as_str(), "audio-volume-high"))
        }
        Command::Text { icon, text } => osd(bus, "showText", &(icon.as_str(), text.as_str())),
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
}
