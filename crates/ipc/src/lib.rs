//! The service and the window talk over a Unix socket, one JSON line per message.
//!
//! Service -> window: [`ServerMessage`]s; a snapshot whenever anything visible changes (and one right after connecting).
//! Window -> service: plain text lines:
//! - `knob CH PERCENT`, `down CH`, `up CH`: on-screen input, same rules as the mixer
//! - `reload`: read config.toml again
//! - `get-config`: answered with [`ServerMessage::Config`]
//! - `config JSON`: save these [`settings::Config`] to config.toml and use them
//! - `token TEXT`: save a new Home Assistant token (readable only by you) and reconnect
//! - `profile NAME`: switch profile
//! - `calibrate R G B`: show the LED calibration pattern with that per-channel correction; `calibrate off` ends it
//! - `entities`: answered with [`ServerMessage::Entities`] (Home Assistant things buttons and knobs can use)

use std::path::PathBuf;

pub mod settings;

use serde::{Deserialize, Serialize};

/// `$XDG_RUNTIME_DIR/cranked-audio.sock`, or `$CRANKED_SOCKET` (tests use their own).
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CRANKED_SOCKET") {
        return PathBuf::from(path);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("cranked-audio.sock")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    Snapshot(Snapshot),
    Config(settings::Config),
    Entities(Vec<Entity>),
    UpdateStatus {
        status: String,
        latest_version: Option<String>,
        can_install: bool,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The USB mixer is connected.
    pub mixer: bool,
    pub profile: String,
    pub profiles: Vec<String>,
    /// Apps playing sound right now (for picking a knob's apps).
    pub playing: Vec<String>,
    /// Every app seen playing (remembered between runs), for picking a knob's apps.
    #[serde(default)]
    pub known_apps: Vec<String>,
    /// Home Assistant is set up.
    pub home_assistant: bool,
    pub knobs: Vec<Knob>,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Knob {
    pub name: String,
    /// What the knob shows, 0..100 (a light knob shows the light's brightness). None until known.
    pub level: Option<u8>,
    /// Colours as `#RRGGBB`.
    pub color: String,
    pub background: String,
    pub muted_color: String,
    pub muted: bool,
    /// Turned down by Auto-lower right now.
    pub lowered: bool,
    /// One line about what this knob controls right now ("Spotify", "Desk Lamp 35%", "Nothing playing").
    pub source: String,
    pub press: String,
    pub hold: String,
    /// The button is being held down.
    pub held: bool,
}
