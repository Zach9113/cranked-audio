//! Settings (`~/.config/cranked-audio/config.toml`), shared by the service and the window.

use std::path::{Path, PathBuf};

use cranked_protocol::Rgb;

/// A profile has 1 to this many knobs (the mixer drives the first five).
pub const MAX_CONTROLS: usize = 16;
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

    /// The knob position that gives `volume` (the other way round from `volume_for`), clamped to the range.
    pub fn knob_for(&self, volume: u8) -> u8 {
        let (low, high) = (self.min.min(self.max), self.max.max(self.min));
        if high == low {
            return 100;
        }
        let span = u32::from(high - low);
        ((u32::from(volume.clamp(low, high) - low) * 100 + span / 2) / span) as u8
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
    /// How the LED the level is passing through shows it.
    pub blend: LedBlend,
}

impl Default for Leds {
    fn default() -> Self {
        Self { brightness: 80, dim_idle: true, gamma: [1.0; 3], blend: LedBlend::default() }
    }
}

/// How a knob's edge LED (the one the level is passing through) goes from the background colour to the level colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedBlend {
    /// Turn Up's fade: the background fades out as the level colour fades in, a little darker through the middle.
    #[default]
    Classic,
    /// An even S-curve mix of the two colours.
    Smooth,
    /// Only the two picked colours, never a mix: the edge LED switches at its middle, dimmed near the switch.
    Clean,
}

/// Calibration sliders go this far either way.
pub const GAMMA_RANGE: (f64, f64) = (0.4, 2.5);

/// Where popups for knob turns and button actions show up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopupStyle {
    /// The Atoll dynamic island when it's running, else KDE's on-screen display, else notifications.
    #[default]
    Auto,
    /// The Atoll dynamic island.
    Island,
    /// KDE Plasma's on-screen display.
    Kde,
    /// Standard desktop notifications (GNOME, Sway, Hyprland, ...).
    Notifications,
}

/// An extra controller besides the 5-knob mixer and the on-screen knobs. Knob numbers count from 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Device {
    /// A volume knob on a keyboard (or any input with volume keys): turning steps a knob, pressing is its button.
    KeyboardKnob {
        /// the input device's name, e.g. "BY Tech Gaming Keyboard Consumer Control"
        name: String,
        knob: usize,
        #[serde(default = "default_step")]
        step: u8,
        /// take the input over so the system volume doesn't move too (its media keys keep working)
        #[serde(default = "yes")]
        grab: bool,
    },
    /// A MIDI controller; each mapped control turns a knob or works as its button.
    Midi {
        /// part of the MIDI port's name, e.g. "nanoKONTROL2"
        name: String,
        #[serde(default)]
        controls: Vec<MidiControl>,
    },
    /// A deej-style Arduino slider box: lines of slider values ("512|1023|0") over USB serial.
    Deej {
        /// e.g. "/dev/ttyACM0" or a /dev/serial/by-id/ path
        port: String,
        #[serde(default = "default_baud")]
        baud: u32,
        /// which knob each slider turns, in order
        knobs: Vec<usize>,
        /// sliders that read backwards
        #[serde(default)]
        invert: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiControl {
    /// MIDI channel 1..16
    pub channel: u8,
    /// "cc" (knobs, faders, buttons sending CC) or "note" (pads and keys)
    pub kind: String,
    pub number: u8,
    pub knob: usize,
    /// works as the knob's button instead of turning it
    #[serde(default)]
    pub button: bool,
    /// an endless encoder sending up/down steps instead of a position
    #[serde(default)]
    pub relative: bool,
}

fn default_step() -> u8 {
    2
}

fn default_baud() -> u32 {
    9600
}

fn yes() -> bool {
    true
}

/// How the window lays out the knobs (the panel widget has the same options in its own settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    /// "knobs" (dials), "sliders" (a compact list) or "faders" (vertical, like a mixing desk)
    pub style: String,
    /// size in percent (30..160)
    pub scale: u8,
    pub show_name: bool,
    pub show_percent: bool,
    pub show_source: bool,
    pub show_button: bool,
    pub show_action_labels: bool,
    /// the button sits in the middle of the dial instead of under it (more compact)
    pub buttons_in_dial: bool,
    /// the window's title bar (name, profile, status, Settings) and the hint line
    pub show_header: bool,
    /// knob numbers (from 1) left out of the window
    pub hidden_knobs: Vec<usize>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            style: "knobs".into(),
            scale: 100,
            show_name: true,
            show_percent: true,
            show_source: true,
            show_button: true,
            show_action_labels: true,
            buttons_in_dial: false,
            show_header: true,
            hidden_knobs: Vec::new(),
        }
    }
}

fn default_theme() -> String {
    "cranked".into()
}

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
    /// Popups for knob turns and button actions.
    pub popups: bool,
    #[serde(default)]
    pub popup_style: PopupStyle,
    /// The window's look: "cranked", "midnight", "graphite", "light" or "system" (your desktop's colours).
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub look: Look,
    /// Controllers besides the mixer and the on-screen knobs.
    #[serde(default, rename = "device")]
    pub devices: Vec<Device>,
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
    popup_style: PopupStyle,
    #[serde(default = "default_theme")]
    theme: String,
    look: Look,
    #[serde(rename = "device")]
    devices: Vec<Device>,
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
            popup_style: PopupStyle::default(),
            theme: default_theme(),
            look: Look::default(),
            devices: Vec::new(),
            leds: Leds::default(),
            home_assistant: HomeAssistant::default(),
            auto_lower: AutoLower::default(),
            channels: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

/// A first-run layout that works out of the box for common Linux apps (matching is by app name or program,
/// case-insensitive, so unknown entries do nothing).
fn default_channels() -> Vec<Channel> {
    let list = |apps: &[&str]| apps.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let knob = |name: &str, apps: Vec<String>, press: Action, color: &str, background: &str| Channel {
        name: name.into(),
        apps,
        press,
        color: color.into(),
        background: background.into(),
        ..Channel::default()
    };
    vec![
        knob(
            "Media",
            list(&["Firefox", "Chromium", "Google Chrome", "Brave", "mpv", "VLC media player", "vlc", "Celluloid"]),
            Action::PlayPause,
            "#FF7A1A",
            "#2A1200",
        ),
        knob(
            "Music",
            list(&["Spotify", "Strawberry", "Elisa", "Rhythmbox", "Amberol", "Tidal", "cider"]),
            Action::Previous,
            "#22C55E",
            "#00230F",
        ),
        knob(
            "Chat",
            list(&["Discord", "Vesktop", "Vencord", "TeamSpeak", "Mumble", "Signal", "Element", "Zoom"]),
            Action::Next,
            "#A855F7",
            "#1A0830",
        ),
        Channel { catch_all: true, ..knob("Game", Vec::new(), Action::SwitchOutput, "#06B6D4", "#002A30") },
        Channel { master: true, ..knob("System", Vec::new(), Action::Mute, "#F4F4F6", "#202024") },
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            popups: true,
            popup_style: PopupStyle::default(),
            theme: default_theme(),
            look: Look::default(),
            devices: Vec::new(),
            leds: Leds::default(),
            home_assistant: HomeAssistant::default(),
            profiles: vec![Profile {
                name: "Default".into(),
                channels: default_channels(),
                auto_lower: AutoLower::default(),
            }],
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
            profiles
                .insert(0, Profile { name: "Default".into(), channels: file.channels, auto_lower: file.auto_lower });
        }
        if profiles.is_empty() {
            profiles = Self::default().profiles;
        }
        let mut config = Self {
            popups: file.popups,
            popup_style: file.popup_style,
            theme: file.theme,
            look: file.look,
            devices: file.devices,
            leds: file.leds,
            home_assistant: file.home_assistant,
            profiles,
        };
        config.normalize();
        Ok(config)
    }

    /// At least one profile, each with exactly one entry per knob.
    pub fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles = Self::default().profiles;
        }
        for profile in &mut self.profiles {
            profile.channels.truncate(MAX_CONTROLS);
            if profile.channels.is_empty() {
                profile.channels.push(Channel::default());
            }
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
        assert_eq!(channel.knob_for(45), 50);
        assert_eq!(channel.knob_for(5), 0);
        assert_eq!(channel.knob_for(100), 100);
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
        assert_eq!(config.profiles[0].channels.len(), 1, "as many knobs as the file lists");
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
    fn devices_read_naturally() {
        let config = Config::parse(
            r##"
            [[device]]
            kind = "keyboard_knob"
            name = "BY Tech Gaming Keyboard Consumer Control"
            knob = 4

            [[device]]
            kind = "midi"
            name = "nanoKONTROL2"
            controls = [
                { channel = 1, kind = "cc", number = 0, knob = 1 },
                { channel = 1, kind = "cc", number = 32, knob = 1, button = true },
            ]

            [[device]]
            kind = "deej"
            port = "/dev/ttyACM0"
            knobs = [1, 2, 3]

            [[channel]]
            name = "Media"
            "##,
        )
        .unwrap();
        assert_eq!(config.devices.len(), 3);
        assert!(matches!(&config.devices[0], Device::KeyboardKnob { knob: 4, step: 2, grab: true, .. }));
        let Device::Midi { controls, .. } = &config.devices[1] else { panic!() };
        assert!(controls[1].button && !controls[0].button);
        assert!(matches!(&config.devices[2], Device::Deej { baud: 9600, invert: false, .. }));
        let again = Config::parse(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(again.devices, config.devices);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#7A2300"), Some((0x7A, 0x23, 0x00)));
        assert_eq!(parse_color("red"), None);
    }
}
