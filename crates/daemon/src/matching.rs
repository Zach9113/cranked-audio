//! Which channel a playback stream belongs to (SPEC.md section 3).

use crate::config::Channel;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stream {
    pub index: u32,
    pub app_name: String,
    pub binary: String,
    pub app_id: String,
    pub media_name: String,
    pub media_role: String,
    pub node_name: String,
    /// Average volume in percent (100 = 0 dB).
    pub volume: f64,
    pub channels: u8,
    pub writable: bool,
    pub muted: bool,
    /// Paused (not playing right now).
    pub corked: bool,
    /// The output device it plays on.
    pub sink: u32,
}

/// Never touched, not even when named in a channel.
const IGNORED: &[&str] = &["nvbroadcast", "nvidia broadcast", "cranked-selftest-ignored"];
/// Never taken by a catch-all channel.
const SYSTEM: &[&str] = &["libcanberra", "plasmashell", "kwin", "xdg-desktop-portal", "org.kde.", "kded"];
const VOICE: &[&str] = &[
    "webrtc voiceengine", "discord", "vesktop", "vencord", "steam voice", "vivox", "teamspeak", "mumble",
    "partychat", "party chat",
];

impl Stream {
    fn names(&self) -> [&str; 5] {
        [&self.app_name, &self.binary, &self.app_id, &self.node_name, &self.media_name]
    }

    fn any_name_contains(&self, markers: &[&str]) -> bool {
        self.names().iter().any(|name| {
            let name = name.to_lowercase();
            markers.iter().any(|marker| name.contains(marker))
        })
    }

    pub fn is_ignored(&self) -> bool {
        self.any_name_contains(IGNORED)
    }

    pub fn is_system_sound(&self) -> bool {
        self.media_role.eq_ignore_ascii_case("event") || self.any_name_contains(SYSTEM)
    }

    pub fn is_voice_chat(&self) -> bool {
        matches!(self.media_role.to_lowercase().as_str(), "phone" | "communication") || self.any_name_contains(VOICE)
    }

    /// 3 = exact app name or program, 2 = exact other name, 1 = the target appears inside the app name.
    fn score(&self, target: &str) -> u8 {
        let target = target.trim().to_lowercase();
        if target.is_empty() {
            return 0;
        }
        let program = self.binary.to_lowercase();
        let program = program.strip_suffix(".exe").unwrap_or(&program);
        let app = self.app_name.to_lowercase();
        if app == target || program == target || program == target.strip_suffix(".exe").unwrap_or(&target) {
            3
        } else if [&self.app_id, &self.node_name].iter().any(|n| n.to_lowercase() == target) {
            2
        } else if app.contains(&target) || program.contains(&target) {
            1
        } else {
            0
        }
    }
}

/// The channel for this stream, if any. The best-scoring named match wins (lowest channel on a tie); otherwise the
/// first catch-all channel, unless it's a system sound or voice chat.
pub fn channel_for(stream: &Stream, channels: &[Channel]) -> Option<usize> {
    if stream.is_ignored() {
        return None;
    }
    let mut best: Option<(u8, usize)> = None;
    for (i, channel) in channels.iter().enumerate().filter(|(_, c)| c.controls_audio() && !c.master) {
        let score = channel.apps.iter().map(|app| stream.score(app)).max().unwrap_or(0);
        if score > 0 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    if let Some((_, i)) = best {
        return Some(i);
    }
    if stream.is_system_sound() || stream.is_voice_chat() {
        return None;
    }
    channels.iter().position(|c| c.catch_all && c.light.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(app: &str, binary: &str) -> Stream {
        Stream { app_name: app.into(), binary: binary.into(), writable: true, ..Stream::default() }
    }

    fn channels() -> Vec<Channel> {
        let named = |apps: &[&str]| Channel { apps: apps.iter().map(|s| s.to_string()).collect(), ..Channel::default() };
        vec![
            named(&["Firefox", "mpv"]),
            named(&["Spotify"]),
            named(&["Vesktop", "Discord"]),
            Channel { catch_all: true, ..named(&["Balatro.exe"]) },
            Channel { master: true, ..Channel::default() },
        ]
    }

    #[test]
    fn named_apps_match_by_name_or_program() {
        let channels = channels();
        assert_eq!(channel_for(&stream("Firefox", "firefox"), &channels), Some(0));
        assert_eq!(channel_for(&stream("spotify", "spotify"), &channels), Some(1));
        assert_eq!(channel_for(&stream("", "Balatro.exe"), &channels), Some(3));
        assert_eq!(channel_for(&stream("WEBRTC VoiceEngine", "vesktop"), &channels), Some(2));
    }

    #[test]
    fn exact_match_beats_a_partial_one() {
        let mut channels = channels();
        channels[0].apps.push("Fire".into());
        channels[1].apps.push("firefox".into());
        assert_eq!(channel_for(&stream("Firefox", "firefox"), &channels), Some(0));
        channels[0].apps = vec!["Fire".into()];
        assert_eq!(channel_for(&stream("Firefox", "firefox"), &channels), Some(1));
    }

    #[test]
    fn catch_all_skips_system_sounds_voice_chat_and_ignored_apps() {
        let channels = channels();
        assert_eq!(channel_for(&stream("Some Game", "game.x86_64"), &channels), Some(3));
        let mut event = stream("Some Game", "game");
        event.media_role = "event".into();
        assert_eq!(channel_for(&event, &channels), None);
        assert_eq!(channel_for(&stream("libcanberra", "plasmashell"), &channels), None);
        assert_eq!(channel_for(&stream("Mumble", "mumble"), &channels), None);
        let mut call = stream("Chromium", "chromium");
        call.media_role = "phone".into();
        assert_eq!(channel_for(&call, &channels), None);
        assert_eq!(channel_for(&stream("nvbroadcast", "nvbroadcast"), &channels), None);
    }

    #[test]
    fn no_catch_all_means_unmatched_streams_are_left_alone() {
        let mut channels = channels();
        channels[3].catch_all = false;
        assert_eq!(channel_for(&stream("Some Game", "game"), &channels), None);
    }
}
