//! Settings (`~/.config/cranked-audio/config.toml`), shared by the service and the window.

use std::path::{Path, PathBuf};

use cranked_protocol::{Rgb, KNOBS};
use serde::{Deserialize, Serialize};

/// What a button does. In the file: `press = "play_pause"` or `hold = { ha_toggle = "light.desk_lamp" }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    Mute,
    PlayPause,
    Next,
    Previous,
    SwitchOutput,
    NextProfile,
    HaToggle(String),
    HaOff(String),
}

impl Action {
    /// Short label for the window; `name_of` turns an entity id into its friendly name.
    pub fn label(&self, name_of: impl Fn(&str) -> String) -> String {
        match self {
            Action::None => "Nothing".into(),
            Action::Mute => "Mute".into(),
            Action::PlayPause => "Play/Pause".into(),
            Action::Next => "Next".into(),
            Action::Previous => "Previous".into(),
            Action::SwitchOutput => "Switch output".into(),
            Action::NextProfile => "Next profile".into(),
            Action::HaToggle(entity) => format!("Toggle {}", name_of(entity)),
            Action::HaOff(entity) => format!("Turn off {}", name_of(entity)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub name: String,
    /// App names to match (application name or program binary, case-insensitive).
    pub apps: Vec<String>,
    /// Also take every playback stream no other channel claims.
    pub catch_all: bool,
    /// Control the default output device instead of apps.
    pub master: bool,
    /// Home Assistant light for this knob (such a knob never touches audio).
    pub light: Option<String>,
    /// The knob's 0..100 maps onto min..max.
    pub min: u8,
    pub max: u8,
    pub press: Action,
    pub hold: Action,
    /// LED colours as `#RRGGBB`: the level fill, the unfilled part, and the whole knob while muted.
    pub color: String,
    pub background: String,
    pub muted_color: String,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            name: String::new(),
            apps: Vec::new(),
            catch_all: false,
            master: false,
            light: None,
            min: 0,
            max: 100,
            press: Action::None,
            hold: Action::Mute,
            color: "#00C8FF".into(),
            background: "#000000".into(),
            muted_color: "#300000".into(),
        }
    }
}

impl Channel {
    pub fn volume_for(&self, knob: u8) -> u8 {
        let (low, high) = (self.min.min(self.max), self.max.max(self.min));
        let span = u32::from(high - low);
        low + ((span * u32::from(knob.min(100)) + 50) / 100) as u8
    }

    pub fn controls_audio(&self) -> bool {
        self.light.is_none() && (self.master || self.catch_all || !self.apps.is_empty())
    }

    pub fn label(&self, index: usize) -> String {
        if self.name.trim().is_empty() {
            format!("Knob {}", index + 1)
        } else {
            self.name.clone()
        }
    }
}

pub fn parse_color(text: &str) -> Option<Rgb> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

/// Turn some knobs down while others play or someone talks in voice chat. Knobs are numbered 1 to 5 here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoLower {
    pub enabled: bool,
    /// Knobs that get turned down.
    pub lower: Vec<usize>,
    /// Knobs whose apps playing sound turn the others down.
    pub triggers: Vec<usize>,
    /// Anyone talking in voice chat (Discord, Steam, TeamSpeak, ...) also counts.
    pub voice_chat: bool,
    /// How far down, in percent of the knob's volume.
    pub amount: u8,
    /// Dim the lowered knobs' LEDs along with the volume.
    pub dim_leds: bool,
}

impl Default for AutoLower {
    fn default() -> Self {
        Self { enabled: false, lower: Vec::new(), triggers: Vec::new(), voice_chat: true, amount: 30, dim_leds: true }
    }
}

impl AutoLower {
    pub fn active(&self) -> bool {
        self.enabled && self.amount > 0 && !self.lower.is_empty()
    }

    pub fn lowers(&self, channel: usize) -> bool {
        self.active() && self.lower.contains(&(channel + 1))
    }

    pub fn triggered_by(&self, channel: usize) -> bool {
        self.active() && self.triggers.contains(&(channel + 1)) && !self.lowers(channel)
    }

    /// The volume factor when fully lowered.
    pub fn low(&self) -> f64 {
        1.0 - f64::from(self.amount.min(100)) / 100.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "channel")]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub auto_lower: AutoLower,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Leds {
    /// 0..100
    pub brightness: u8,
    /// Dim a knob to a quarter when none of its apps are playing.
    pub dim_idle: bool,
    /// Per-channel (red, green, blue) correction for dimming, set with the LED calibration: an LED's three colours
    /// don't dim evenly, so a dimmed colour drifts in tint unless each channel is corrected. 1.0 = no correction;
    /// higher keeps that channel brighter when dimmed.
    pub gamma: [f64; 3],
}

impl Default for Leds {
    fn default() -> Self {
        Self { brightness: 80, dim_idle: true, gamma: [1.0; 3] }
    }
}

/// Calibration sliders go this far either way.
pub const GAMMA_RANGE: (f64, f64) = (0.4, 2.5);

/// Off until `url` is set. The token is a long-lived access token, kept in its own file readable only by you.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeAssistant {
    pub url: Option<String>,
    /// Default: `ha-token` next to config.toml.
    pub token_file: Option<PathBuf>,
}

impl Config {
    /// Every Home Assistant entity any profile uses (lights on knobs, entities on buttons).
    pub fn ha_entities(&self) -> Vec<String> {
        let mut entities: Vec<String> = Vec::new();
        for channel in self.profiles.iter().flat_map(|p| &p.channels) {
            let actions = [&channel.press, &channel.hold];
            let from_buttons = actions.into_iter().filter_map(|a| match a {
                Action::HaToggle(e) | Action::HaOff(e) => Some(e.clone()),
                _ => None,
            });
            for entity in channel.light.iter().cloned().chain(from_buttons) {
                if !entity.is_empty() && !entities.contains(&entity) {
                    entities.push(entity);
                }
            }
        }
        entities
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// KDE on-screen popups for knob turns and button actions.
    pub popups: bool,
    pub leds: Leds,
    pub home_assistant: HomeAssistant,
    #[serde(rename = "profile")]
    pub profiles: Vec<Profile>,
}

/// The file may list `[[channel]]` directly (one profile) or `[[profile]]` sections with their own channels.
#[derive(Deserialize)]
#[serde(default)]
struct File {
    popups: bool,
    leds: Leds,
    home_assistant: HomeAssistant,
    auto_lower: AutoLower,
    #[serde(rename = "channel")]
    channels: Vec<Channel>,
    #[serde(rename = "profile")]
    profiles: Vec<Profile>,
}

impl Default for File {
    fn default() -> Self {
        Self {
            popups: true,
            leds: Leds::default(),
            home_assistant: HomeAssistant::default(),
            auto_lower: AutoLower::default(),
            channels: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

fn default_channels() -> Vec<Channel> {
    let names = ["Media", "Music", "Chat", "Game", "System"];
    let presses = [Action::PlayPause, Action::Previous, Action::Next, Action::SwitchOutput, Action::Mute];
    names
        .iter()
        .zip(presses)
        .enumerate()
        .map(|(i, (name, press))| Channel {
            name: (*name).into(),
            catch_all: i == 3,
            master: i == 4,
            press,
            ..Channel::default()
        })
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            popups: true,
            leds: Leds::default(),
            home_assistant: HomeAssistant::default(),
            profiles: vec![Profile { name: "Default".into(), channels: default_channels(), auto_lower: AutoLower::default() }],
        }
    }
}

pub fn base_dir(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cranked-audio")
}

impl Config {
    pub fn default_path() -> PathBuf {
        base_dir("XDG_CONFIG_HOME", ".config").join("config.toml")
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut profiles = file.profiles;
        if !file.channels.is_empty() {
            profiles.insert(0, Profile { name: "Default".into(), channels: file.channels, auto_lower: file.auto_lower });
        }
        if profiles.is_empty() {
            profiles = Self::default().profiles;
        }
        let mut config = Self { popups: file.popups, leds: file.leds, home_assistant: file.home_assistant, profiles };
        config.normalize();
        Ok(config)
    }

    /// At least one profile, each with exactly one entry per knob.
    pub fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles = Self::default().profiles;
        }
        for profile in &mut self.profiles {
            profile.channels.resize_with(KNOBS, Channel::default);
        }
    }

    /// Writes the file in one step (a crash mid-write never leaves half a file).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("toml.new");
        std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path)).map_err(|e| e.to_string())
    }

    /// Where the Home Assistant token is kept.
    pub fn token_path(&self, config_path: &Path) -> PathBuf {
        self.home_assistant.token_file.clone().unwrap_or_else(|| config_path.with_file_name("ha-token"))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn profile_index(&self, name: &str) -> usize {
        self.profiles.iter().position(|p| p.name == name).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_maps_onto_the_volume_range() {
        let channel = Channel { min: 10, max: 80, ..Channel::default() };
        assert_eq!(channel.volume_for(0), 10);
        assert_eq!(channel.volume_for(50), 45);
        assert_eq!(channel.volume_for(100), 80);
    }

    #[test]
    fn toml_round_trip() {
        let config = Config::default();
        let text = toml::to_string(&config).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), config);
    }

    #[test]
    fn plain_channels_are_one_profile_and_actions_read_naturally() {
        let config = Config::parse(
            r##"
            [[channel]]
            name = "Lamp"
            light = "light.desk_lamp"
            press = { ha_toggle = "switch.fan" }
            hold = "none"

            [[profile]]
            name = "Gaming"
            [[profile.channel]]
            apps = ["Steam"]
            press = "next_profile"
            "##,
        )
        .unwrap();
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.profiles[0].name, "Default");
        assert_eq!(config.profiles[0].channels.len(), KNOBS);
        assert_eq!(config.profiles[0].channels[0].press, Action::HaToggle("switch.fan".into()));
        assert_eq!(config.profiles[0].channels[0].hold, Action::None);
        assert_eq!(config.profiles[1].channels[0].press, Action::NextProfile);
        assert_eq!(config.profiles[1].channels[0].hold, Action::Mute);
        assert_eq!(config.profile_index("Gaming"), 1);
        assert_eq!(config.profile_index("gone"), 0);
    }

    #[test]
    fn home_assistant_entities_are_collected_once() {
        let config = Config::parse(
            r##"
            [home_assistant]
            url = "http://ha.local:8123/"
            [[channel]]
            light = "light.desk_lamp"
            press = { ha_toggle = "switch.coffee_maker" }
            hold = { ha_toggle = "light.desk_lamp" }
            "##,
        )
        .unwrap();
        assert_eq!(config.ha_entities(), ["light.desk_lamp", "switch.coffee_maker"]);
        assert!(Config::default().ha_entities().is_empty());
    }

    #[test]
    fn auto_lower_settings() {
        let config = Config::parse(
            r##"
            [auto_lower]
            enabled = true
            lower = [1, 2]
            triggers = [2, 3]
            [[channel]]
            name = "Media"
            "##,
        )
        .unwrap();
        let auto = &config.profiles[0].auto_lower;
        assert!(auto.lowers(0) && auto.lowers(1) && !auto.lowers(2));
        assert!(auto.triggered_by(2) && !auto.triggered_by(1), "a lowered knob never triggers");
        assert!((auto.low() - 0.7).abs() < 1e-9);
        assert!(!AutoLower { amount: 0, ..auto.clone() }.active());
    }

    #[test]
    fn saved_settings_read_back_the_same() {
        let mut config = Config::parse(
            r##"
            [home_assistant]
            url = "http://ha.local:8123"
            [auto_lower]
            enabled = true
            lower = [1]
            [[channel]]
            name = "Lamp"
            light = "light.desk_lamp"
            press = { ha_toggle = "switch.fan" }
            "##,
        )
        .unwrap();
        config.profiles.push(Profile { name: "Two".into(), ..config.profiles[0].clone() });
        let dir = std::env::temp_dir().join(format!("cranked-settings-{}", std::process::id()));
        let path = dir.join("config.toml");
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        let json = serde_json::to_string(&config).unwrap();
        let mut from_window: Config = serde_json::from_str(&json).unwrap();
        from_window.normalize();
        assert_eq!(from_window, config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#7A2300"), Some((0x7A, 0x23, 0x00)));
        assert_eq!(parse_color("red"), None);
    }
}
