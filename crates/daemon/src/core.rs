//! The rules, with no I/O: events in, things to do out (SPEC.md sections 2 to 5 and 8).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use cranked_ipc::settings::MAX_CONTROLS as MAX;
use cranked_protocol::{led_frame, raw_to_percent, Decoder, Event as Input, KnobFilter, KNOBS};

use crate::audio::Command as Audio;
use crate::buttons::{Buttons, Gesture};
use crate::config::{parse_color, Action, AutoLower, Channel, Config, State};
use crate::desktop::{Command as Desktop, MediaKey};
use crate::duck::Ducker;
use crate::ha::{Command as Ha, EntityState};
use crate::leds::{calibration, colors, KnobLook};
use crate::matching::{channel_for, Stream};
use crate::Event;

const SYNC_QUIET: Duration = Duration::from_millis(300);
const SYNC_LIMIT: Duration = Duration::from_millis(2500);
const APPLY_EVERY: Duration = Duration::from_millis(50);
/// Fastest LED frame rate (25 per second): smooth enough for the easing below, a 48-byte frame takes ~4 ms.
const LED_EVERY: Duration = Duration::from_millis(40);
/// LEDs glide toward a new level with this time constant instead of stepping.
const LED_EASE: f64 = 0.07;
/// The mixer blanks its LEDs soon after a frame, so the current one is sent again this often.
const LED_REFRESH: Duration = Duration::from_millis(250);
/// A button's "Turning on ..." popup is followed by the result if it arrives within this time.
const RESULT_WITHIN: Duration = Duration::from_secs(10);
/// Without a mixer by then, the knobs start where they were last time.
const NO_MIXER_AFTER: Duration = Duration::from_secs(4);
/// Knob positions are saved once they've been still this long.
const SAVE_AFTER: Duration = Duration::from_secs(2);
/// A calibration nobody touches for this long ends by itself (say the window was closed).
const CALIBRATION_TIMEOUT: Duration = Duration::from_secs(180);
/// How many seen app names to remember.
const SEEN_APPS: usize = 200;
/// While the screen is off, touching a knob or button lights the LEDs for this long.
const WAKE_FOR: Duration = Duration::from_secs(30);
/// After the mixer connects, knob readings don't move lights for this long (it sends warm-up readings).
const LIGHTS_SETTLE: Duration = Duration::from_secs(4);
/// A light that's off only comes on when its knob is turned up at least this far (a wobble never switches it on).
const LIGHT_ON_MOVE: u8 = 3;
/// Knob and light this close together move 1:1 (SPEC.md section 7).
const PICKUP_TOLERANCE: i32 = 3;
/// A stream more than this many points off its channel gets put back (fast guard).
const GUARD_TOLERANCE: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Audio(Audio),
    Desktop(Desktop),
    Leds([u8; 48]),
    SaveState(State),
    Ha(Ha),
}

/// The light's new brightness when its knob moves from `previous` to `knob`. If the light was changed elsewhere,
/// the knob catches up proportionally instead of jumping: the ends still mean off and full.
pub fn catch_up(light: u8, previous: u8, knob: u8) -> u8 {
    let (light, previous, knob) = (i32::from(light.min(100)), i32::from(previous.min(100)), i32::from(knob.min(100)));
    if knob == previous {
        return light as u8;
    }
    if knob == 0 || knob == 100 || (light - previous).abs() <= PICKUP_TOLERANCE {
        return knob as u8;
    }
    let moved = if knob > previous {
        light + ((knob - previous) * (100 - light) + (100 - previous) / 2) / (100 - previous)
    } else {
        light - ((previous - knob) * light + previous / 2) / previous
    };
    moved.clamp(0, 100) as u8
}

struct StartupSync {
    started: Instant,
    last_reading: Instant,
    seen: [bool; KNOBS],
}

pub struct Core {
    config: Config,
    profile: usize,
    mixer: Decoder,
    on_screen: Decoder,
    filter: KnobFilter,
    sync: Option<StartupSync>,
    mixer_connected: bool,
    knobs: [Option<u8>; MAX],
    dirty: [bool; MAX],
    popup: [bool; MAX],
    muted: [bool; MAX],
    buttons: Buttons,
    last_apply: Option<Instant>,
    last_frame: Option<[u8; 48]>,
    last_led_send: Option<Instant>,
    led_waiting: bool,
    /// The level each knob's LEDs show right now (eases toward the real level).
    shown: [Option<f64>; MAX],
    shown_at: Option<Instant>,
    led_animating: bool,
    streams: HashMap<u32, (Stream, Option<usize>)>,
    entities: HashMap<String, EntityState>,
    /// Knob position the light was last moved from.
    applied: [Option<u8>; MAX],
    awaiting: HashMap<String, Instant>,
    ducker: Ducker,
    ducking: bool,
    /// Streams whose level is being watched for Auto-lower, with the output they were on.
    metered: HashMap<u32, u32>,
    started: Instant,
    /// Home Assistant is set up (shown in the window).
    pub ha_enabled: bool,
    saved: [Option<u8>; MAX],
    /// Apps seen playing (for the window's app list), newest last.
    seen_apps: Vec<String>,
    restored: bool,
    moved_at: Option<Instant>,
    screen_off: bool,
    touched_at: Option<Instant>,
    /// LED calibration in progress: when it last changed, and the correction being tried.
    calibrating: Option<(Instant, [f64; 3])>,
    /// Lights ignore the mixer's knobs until then (see LIGHTS_SETTLE).
    lights_settle_until: Option<Instant>,
    /// The time of the current tick.
    now: Option<Instant>,
    pub verbose: bool,
}

impl Core {
    pub fn new(config: Config, state: &State) -> Self {
        Self::starting_at(config, state, Instant::now())
    }

    pub fn starting_at(config: Config, state: &State, started: Instant) -> Self {
        let profile = config.profile_index(&state.profile);
        let saved = std::array::from_fn(|i| state.knobs.get(&(i + 1).to_string()).map(|p| (*p).min(100)));
        let seen_apps = state.apps.iter().filter(|n| !n.starts_with("cranked-selftest")).cloned().collect();
        let mut core = Self {
            config,
            profile,
            mixer: Decoder::new(),
            on_screen: Decoder::new(),
            filter: KnobFilter::new(),
            sync: None,
            mixer_connected: false,
            knobs: [None; MAX],
            dirty: [false; MAX],
            popup: [false; MAX],
            muted: [false; MAX],
            buttons: Buttons::default(),
            last_apply: None,
            last_frame: None,
            last_led_send: None,
            led_waiting: false,
            shown: [None; MAX],
            shown_at: None,
            led_animating: false,
            streams: HashMap::new(),
            entities: HashMap::new(),
            applied: [None; MAX],
            awaiting: HashMap::new(),
            ducker: Ducker::default(),
            ducking: false,
            metered: HashMap::new(),
            started,
            ha_enabled: false,
            saved,
            seen_apps,
            restored: false,
            moved_at: None,
            screen_off: false,
            touched_at: None,
            calibrating: None,
            lights_settle_until: None,
            now: None,
            verbose: false,
        };
        core.update_holds();
        core
    }

    /// How many knobs the current profile has.
    fn count(&self) -> usize {
        self.channels().len().min(MAX)
    }

    fn channels(&self) -> &[Channel] {
        &self.config.profiles[self.profile].channels
    }

    fn auto_lower(&self) -> &AutoLower {
        &self.config.profiles[self.profile].auto_lower
    }

    /// Only what can trigger Auto-lower is metered: trigger knobs' apps and voice chat, never a lowered knob.
    fn wants_meter(&self, stream: &Stream, channel: Option<usize>) -> bool {
        let auto = self.auto_lower();
        if !auto.active() || stream.is_ignored() {
            return false;
        }
        match channel {
            Some(c) if auto.lowers(c) => false,
            Some(c) if auto.triggered_by(c) => true,
            _ => auto.voice_chat && stream.is_voice_chat(),
        }
    }

    fn sync_meter(&mut self, index: u32, out: &mut Vec<Out>) {
        let wanted = self.streams.get(&index).filter(|(s, c)| self.wants_meter(s, *c)).map(|(s, _)| s.sink);
        let current = self.metered.get(&index).copied();
        if wanted == current {
            return;
        }
        if let Some(sink) = current {
            self.metered.remove(&index);
            out.push(Out::Audio(Audio::Meter { index, sink, on: false }));
        }
        if let Some(sink) = wanted {
            if self.verbose {
                eprintln!("auto-lower: watching stream {index}");
            }
            self.metered.insert(index, sink);
            out.push(Out::Audio(Audio::Meter { index, sink, on: true }));
        }
    }

    fn update_holds(&mut self) {
        let holds: Vec<bool> = self.channels().iter().map(|c| c.hold != Action::None).collect();
        for (slot, has) in self.buttons.has_hold.iter_mut().zip(holds) {
            *slot = has;
        }
    }

    /// Use new settings: streams are matched again and the knobs re-applied where they are (lights stay put).
    pub fn set_config(&mut self, config: Config, out: &mut Vec<Out>) {
        let current = self.config.profiles[self.profile].name.clone();
        self.config = config;
        self.profile = self.config.profile_index(&current);
        self.update_holds();
        let channels = self.config.profiles[self.profile].channels.clone();
        for (stream, channel) in self.streams.values_mut() {
            *channel = channel_for(stream, &channels);
        }
        let indexes: Vec<u32> = self.streams.keys().chain(self.metered.keys()).copied().collect();
        for index in indexes {
            self.sync_meter(index, out);
        }
        for channel in 0..self.count() {
            if let Some(percent) = self.knobs[channel] {
                self.set_knob(channel, percent, false);
            }
        }
        self.last_frame = None;
    }

    /// Everything the window shows.
    pub fn snapshot(&self) -> cranked_ipc::Snapshot {
        let name_of = |entity: &str| self.entities.get(entity).map_or_else(|| entity.to_string(), |s| s.name.clone());
        let knobs = self
            .channels()
            .iter()
            .enumerate()
            .map(|(channel, config)| {
                let source = if let Some(entity) = &config.light {
                    match self.entities.get(entity) {
                        Some(state) if state.is_on() => format!("{} {}%", state.name, state.brightness.unwrap_or(100)),
                        Some(state) => format!("{} {}", state.name, state.state),
                        None => entity.clone(),
                    }
                } else if config.master {
                    "Output volume".into()
                } else {
                    let playing = self
                        .streams
                        .values()
                        .filter(|(s, c)| *c == Some(channel) && !s.corked)
                        .max_by_key(|(s, _)| s.index)
                        .map(|(s, _)| if s.app_name.is_empty() { s.binary.clone() } else { s.app_name.clone() });
                    match playing {
                        Some(app) => app,
                        None if config.controls_audio() => "Nothing playing".into(),
                        None => "Not set up".into(),
                    }
                };
                cranked_ipc::Knob {
                    name: config.label(channel),
                    level: self.level(channel),
                    color: config.color.clone(),
                    background: config.background.clone(),
                    muted_color: config.muted_color.clone(),
                    muted: self.muted[channel],
                    lowered: self.auto_lower().lowers(channel) && self.ducker.factor() < 1.0,
                    source,
                    press: config.press.label(name_of),
                    hold: config.hold.label(name_of),
                    held: self.buttons.is_down(channel),
                }
            })
            .collect();
        let mut playing: Vec<String> = self
            .streams
            .values()
            .filter(|(s, _)| !s.is_ignored() && !s.is_system_sound())
            .map(|(s, _)| if s.app_name.is_empty() { s.binary.clone() } else { s.app_name.clone() })
            .filter(|name| !name.is_empty())
            .collect();
        playing.sort_by_key(|n| n.to_lowercase());
        playing.dedup();
        let mut known_apps = self.seen_apps.clone();
        known_apps.sort_by_key(|n| n.to_lowercase());
        known_apps.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        cranked_ipc::Snapshot {
            mixer: self.mixer_connected,
            profile: self.config.profiles[self.profile].name.clone(),
            profiles: self.config.profiles.iter().map(|p| p.name.clone()).collect(),
            playing,
            known_apps,
            home_assistant: self.ha_enabled,
            knobs,
            version: format!("v{}", env!("CARGO_PKG_VERSION")),
        }
    }

    pub fn knob(&self, channel: usize) -> Option<u8> {
        self.knobs.get(channel).copied().flatten()
    }

    #[cfg(test)]
    pub fn is_syncing(&self) -> bool {
        self.sync.is_some()
    }

    fn has_pending(&self) -> bool {
        self.dirty.iter().any(|d| *d)
    }

    /// How long the caller may sleep before calling `tick`.
    pub fn next_wake(&self) -> Duration {
        if self.led_waiting || self.led_animating {
            LED_EVERY.min(APPLY_EVERY)
        } else if self.sync.is_some() || self.has_pending() || self.buttons.any_down() || self.ducking {
            APPLY_EVERY
        } else if self.mixer_connected && !(self.screen_off && self.touched_at.is_none_or(|t| t.elapsed() >= WAKE_FOR))
        {
            LED_REFRESH
        } else {
            Duration::from_secs(1)
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        match event {
            Event::MixerConnected => {
                self.restored = true;
                self.mixer = Decoder::new();
                self.mixer_connected = true;
                self.last_frame = None;
                self.sync = Some(StartupSync { started: now, last_reading: now, seen: [false; KNOBS] });
            }
            Event::MixerLost => {
                self.mixer_connected = false;
                self.sync = None;
            }
            Event::ScreenOff(off) => {
                self.screen_off = off;
                if self.verbose {
                    eprintln!("screen {}", if off { "off: LEDs dark" } else { "on" });
                }
            }
            Event::Bytes(bytes) => {
                for input in self.mixer.push(&bytes) {
                    self.mixer_input(input, now, &mut out);
                }
            }
            Event::OnScreen(bytes) => {
                for input in self.on_screen.push(&bytes) {
                    match input {
                        // on-screen knobs can't glitch, so they skip the spike filter
                        Input::Knob { channel, raw } => {
                            let channel = usize::from(channel);
                            // the mixer's filter tracks its own five knobs; the others are on screen only
                            let percent =
                                self.filter.seed(channel, raw).or_else(|| (raw <= 1023).then(|| raw_to_percent(raw)));
                            if let Some(percent) = percent {
                                self.set_knob(channel, percent, true);
                            }
                        }
                        other => self.button_input(other, now, &mut out),
                    }
                }
            }
            Event::Stream(stream) => {
                let channel = channel_for(&stream, self.channels());
                if self.verbose && !self.streams.contains_key(&stream.index) {
                    eprintln!(
                        "stream {} {:?} ({}) -> channel {:?}",
                        stream.index, stream.app_name, stream.binary, channel
                    );
                }
                if let Some(c) = channel {
                    out.extend(self.guard(&stream, c));
                }
                let index = stream.index;
                if !self.streams.contains_key(&index) {
                    self.remember_app(&stream);
                }
                self.streams.insert(index, (stream, channel));
                self.sync_meter(index, &mut out);
            }
            Event::StreamRemoved(index) => {
                self.streams.remove(&index);
                self.sync_meter(index, &mut out);
            }
            Event::Loud(index) => {
                if self.streams.get(&index).is_some_and(|(s, c)| self.wants_meter(s, *c)) {
                    self.ducker.loud(now);
                    self.ducking = true;
                }
            }
            Event::OutputSwitched(name) => {
                let text = name.unwrap_or_else(|| "No other output".into());
                out.extend(self.popup_text("audio-card", text));
            }
            Event::HaState(state) => {
                if self.awaiting.remove(&state.entity).is_some_and(|asked| now.duration_since(asked) < RESULT_WITHIN) {
                    let word = if state.is_on() { "on" } else { "off" };
                    out.extend(self.popup_text("home", format!("{} {word}", state.name)));
                }
                if self.verbose && self.entities.get(&state.entity) != Some(&state) {
                    eprintln!("ha: {} is {} {:?}", state.entity, state.state, state.brightness);
                }
                self.entities.insert(state.entity.clone(), state);
            }
            Event::HaFailed => out.extend(self.popup_text("dialog-warning", "Home Assistant didn't respond".into())),
            Event::AudioReady => {}
            Event::AudioLost => {
                self.streams.clear();
                self.metered.clear();
            }
            Event::ClientConnected(_)
            | Event::ClientLine(_)
            | Event::Tray(_)
            | Event::HaEntities(_)
            | Event::UpdateCheckResult(_)
            | Event::UpdateApplyResult(_)
            | Event::Remote(_)
            | Event::Device(_)
            | Event::MidiLearned(_) => {}
        }
        out
    }

    /// Remembers a newly seen app's name for the window's app list (saved with the other state).
    fn remember_app(&mut self, stream: &Stream) {
        if stream.is_ignored() || stream.is_system_sound() {
            return;
        }
        let name = if stream.app_name.is_empty() { stream.binary.clone() } else { stream.app_name.clone() };
        // the live tests' throwaway streams
        if name.trim().is_empty() || name.starts_with("cranked-selftest") {
            return;
        }
        let before = self.seen_apps.len();
        self.seen_apps.retain(|n| !n.eq_ignore_ascii_case(&name));
        let is_new = before == self.seen_apps.len();
        self.seen_apps.push(name);
        if self.seen_apps.len() > SEEN_APPS {
            self.seen_apps.remove(0);
        }
        if is_new {
            // saved with the knob positions once things are still
            self.moved_at.get_or_insert_with(Instant::now);
        }
    }

    /// Start or adjust (`Some`) or end (`None`) the LED calibration pattern, trying this per-channel correction.
    pub fn calibrate(&mut self, gamma: Option<[f64; 3]>, now: Instant) {
        let (low, high) = cranked_ipc::settings::GAMMA_RANGE;
        self.calibrating = gamma.map(|g| (now, g.map(|c| c.clamp(low, high))));
        self.last_frame = None;
    }

    /// LEDs stay dark while the screen is off, unless the mixer was just touched.
    fn leds_dark(&self, now: Instant) -> bool {
        self.calibrating.is_none()
            && self.screen_off
            && !self.touched_at.is_some_and(|t| now.saturating_duration_since(t) < WAKE_FOR)
    }

    fn mixer_input(&mut self, input: Input, now: Instant, out: &mut Vec<Out>) {
        if self.sync.is_none() {
            self.touched_at = Some(now);
        }
        let Input::Knob { channel, raw } = input else {
            return self.button_input(input, now, out);
        };
        let channel = usize::from(channel);
        if let Some(sync) = &mut self.sync {
            if self.filter.seed(channel, raw).is_some() {
                sync.seen[channel] = true;
                sync.last_reading = now;
            }
        } else if let Some(percent) = self.filter.feed(channel, raw) {
            self.set_knob(channel, percent, true);
        }
    }

    fn button_input(&mut self, input: Input, now: Instant, out: &mut Vec<Out>) {
        match input {
            Input::ButtonDown { channel } => self.buttons.down(usize::from(channel), now),
            Input::ButtonUp { channel } => {
                if let Some(gesture) = self.buttons.up(usize::from(channel), now) {
                    self.gesture(gesture, now, out);
                }
            }
            Input::Knob { .. } => {}
        }
    }

    /// Knobs from `first` on that aren't known yet start where they were saved (lights are left alone).
    fn restore_saved(&mut self, first: usize) {
        for channel in first..self.count() {
            if let (None, Some(percent)) = (self.knobs[channel], self.saved[channel]) {
                let raw = cranked_protocol::percent_to_raw(percent);
                self.filter.seed(channel, raw);
                self.set_knob(channel, percent, false);
            }
        }
    }

    fn state(&self) -> State {
        let knobs = (0..MAX).filter_map(|i| Some(((i + 1).to_string(), self.knobs[i]?))).collect();
        State { profile: self.config.profiles[self.profile].name.clone(), knobs, apps: self.seen_apps.clone() }
    }

    fn set_knob(&mut self, channel: usize, percent: u8, popup: bool) {
        if channel < self.count() {
            if popup {
                self.moved_at = Some(Instant::now());
            }
            self.knobs[channel] = Some(percent);
            self.dirty[channel] = true;
            self.popup[channel] |= popup;
        }
    }

    fn popup_text(&self, icon: &str, text: String) -> Option<Out> {
        let style = self.config.popup_style;
        self.config.popups.then(|| Out::Desktop(Desktop::Text { icon: icon.into(), text, style }))
    }

    fn gesture(&mut self, gesture: Gesture, now: Instant, out: &mut Vec<Out>) {
        let (channel, hold) = match gesture {
            Gesture::Press(c) => (c, false),
            Gesture::Hold(c) => (c, true),
        };
        // a button on a knob this profile doesn't have does nothing
        let Some(config) = self.channels().get(channel) else { return };
        let action = if hold { config.hold.clone() } else { config.press.clone() };
        if self.verbose {
            eprintln!("button {channel}: {gesture:?} -> {action:?}");
        }
        match action {
            Action::None => {}
            Action::Mute => self.toggle_mute(channel, out),
            Action::PlayPause => out.push(Out::Desktop(Desktop::Media(MediaKey::PlayPause))),
            Action::Next => out.push(Out::Desktop(Desktop::Media(MediaKey::Next))),
            Action::Previous => out.push(Out::Desktop(Desktop::Media(MediaKey::Previous))),
            Action::SwitchOutput => out.push(Out::Audio(Audio::SwitchOutput)),
            Action::NextProfile => self.next_profile(out),
            Action::HaToggle(entity) => self.ha_button("toggle", entity, now, out),
            Action::HaOff(entity) => self.ha_button("turn_off", entity, now, out),
        }
    }

    fn ha_button(&mut self, service: &'static str, entity: String, now: Instant, out: &mut Vec<Out>) {
        let known = self.entities.get(&entity);
        let name = known.map_or_else(|| entity.clone(), |s| s.name.clone());
        let text = match (service, known.map(EntityState::is_on)) {
            ("turn_off", _) | (_, Some(true)) => format!("Turning off {name}"),
            (_, Some(false)) => format!("Turning on {name}"),
            _ => format!("Toggling {name}"),
        };
        out.extend(self.popup_text("home", text));
        self.awaiting.insert(entity.clone(), now);
        out.push(Out::Ha(Ha::Call { service, entity }));
    }

    fn mute_commands(&self, channel: usize, mute: bool) -> Vec<Out> {
        let mut out: Vec<Out> = self
            .streams
            .values()
            .filter(|(stream, c)| *c == Some(channel) && stream.muted != mute)
            .map(|(stream, _)| Out::Audio(Audio::MuteStream { index: stream.index, mute }))
            .collect();
        if self.channels()[channel].master {
            out.push(Out::Audio(Audio::MuteMaster { mute }));
        }
        out
    }

    fn toggle_mute(&mut self, channel: usize, out: &mut Vec<Out>) {
        if !self.channels().get(channel).is_some_and(Channel::controls_audio) {
            return;
        }
        let mute = !self.muted[channel];
        self.muted[channel] = mute;
        out.extend(self.mute_commands(channel, mute));
        let label = self.channels()[channel].label(channel);
        let (icon, word) = if mute { ("audio-volume-muted", "muted") } else { ("audio-volume-high", "unmuted") };
        out.extend(self.popup_text(icon, format!("{label} {word}")));
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn switch_profile(&mut self, name: &str, out: &mut Vec<Out>) {
        if let Some(index) = self.config.profiles.iter().position(|p| p.name == name) {
            if index != self.profile {
                self.use_profile(index, out);
            }
        }
    }

    fn next_profile(&mut self, out: &mut Vec<Out>) {
        self.use_profile((self.profile + 1) % self.config.profiles.len(), out);
    }

    fn use_profile(&mut self, index: usize, out: &mut Vec<Out>) {
        for channel in 0..self.count() {
            if std::mem::take(&mut self.muted[channel]) {
                out.extend(self.mute_commands(channel, false));
            }
        }
        self.profile = index.min(self.config.profiles.len() - 1);
        self.update_holds();
        let channels = self.config.profiles[self.profile].channels.clone();
        for (stream, channel) in self.streams.values_mut() {
            *channel = channel_for(stream, &channels);
        }
        let indexes: Vec<u32> = self.streams.keys().chain(self.metered.keys()).copied().collect();
        for index in indexes {
            self.sync_meter(index, out);
        }
        for channel in 0..self.count() {
            if let Some(percent) = self.knobs[channel] {
                self.set_knob(channel, percent, false);
            }
        }
        let name = self.config.profiles[self.profile].name.clone();
        out.push(Out::SaveState(self.state()));
        out.extend(self.popup_text("configure", format!("Profile: {name}")));
    }

    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        self.now = Some(now);
        if self.calibrating.is_some_and(|(at, _)| now.saturating_duration_since(at) >= CALIBRATION_TIMEOUT) {
            self.calibrate(None, now);
        }
        for gesture in self.buttons.tick(now) {
            self.gesture(gesture, now, &mut out);
        }
        if !self.restored && !self.mixer_connected && now.duration_since(self.started) >= NO_MIXER_AFTER {
            self.restored = true;
            self.restore_saved(0);
            if self.verbose {
                eprintln!("no mixer: knobs start at {:?}", self.knobs);
            }
        }
        if self.moved_at.is_some_and(|t| now.saturating_duration_since(t) >= SAVE_AFTER) {
            self.moved_at = None;
            out.push(Out::SaveState(self.state()));
        }
        if let Some(sync) = &self.sync {
            let all_seen = sync.seen.iter().all(|s| *s);
            let quiet = now.duration_since(sync.last_reading) >= SYNC_QUIET;
            if (all_seen && quiet) || now.duration_since(sync.started) >= SYNC_LIMIT {
                self.sync = None;
                self.lights_settle_until = Some(now + LIGHTS_SETTLE);
                // the mixer's own knobs
                for channel in 0..KNOBS.min(self.count()) {
                    if let Some(percent) = self.filter.position(channel) {
                        self.set_knob(channel, percent, false);
                    }
                }
                // knobs past the mixer's (on screen, keyboard knobs...) start where they were
                self.restore_saved(KNOBS);
                if self.verbose {
                    eprintln!("startup sync done: {:?}", self.knobs);
                }
            }
        }
        if self.ducking {
            let low = if self.auto_lower().active() { self.auto_lower().low() } else { 1.0 };
            if self.ducker.update(now, low) {
                for channel in 0..self.count() {
                    if self.auto_lower().lowers(channel) || !self.auto_lower().active() {
                        self.dirty[channel] |= self.knobs[channel].is_some();
                    }
                }
            }
            self.ducking = self.ducker.busy(now);
        }
        if self.has_pending() && self.last_apply.is_none_or(|t| now.duration_since(t) >= APPLY_EVERY) {
            self.last_apply = Some(now);
            for channel in 0..self.count() {
                if std::mem::take(&mut self.dirty[channel]) {
                    let turned = std::mem::take(&mut self.popup[channel]);
                    out.extend(self.apply(channel, turned));
                    if turned && self.config.popups {
                        out.extend(self.knob_popup(channel));
                    }
                }
            }
        }
        out.extend(self.leds(now));
        out
    }

    fn knob_popup(&self, channel: usize) -> Option<Out> {
        let config = &self.channels()[channel];
        let percent = if config.controls_audio() { self.target(channel)? } else { self.level(channel)? };
        Some(Out::Desktop(Desktop::Volume { percent, name: config.label(channel), style: self.config.popup_style }))
    }

    /// What the knob shows: the light's brightness for a light knob, otherwise the knob position.
    fn level(&self, channel: usize) -> Option<u8> {
        let light = self.channels()[channel].light.as_ref().and_then(|e| self.entities.get(e)?.brightness);
        light.or_else(|| self.knob(channel))
    }

    fn leds(&mut self, now: Instant) -> Option<Out> {
        if !self.mixer_connected || self.sync.is_some() {
            return None;
        }
        if self.leds_dark(now) {
            // the board blanks its LEDs by itself shortly after the last frame; one dark frame makes it immediate
            let dark = led_frame(&[[(0, 0, 0); cranked_protocol::LEDS_PER_KNOB]; KNOBS]);
            self.led_waiting = false;
            if self.last_frame == Some(dark) {
                return None;
            }
            self.last_frame = Some(dark);
            self.last_led_send = Some(now);
            return Some(Out::Leds(dark));
        }
        self.ease_levels(now);
        let looks: [KnobLook; KNOBS] = std::array::from_fn(|channel| {
            let Some(config) = self.channels().get(channel) else {
                return KnobLook {
                    level: None,
                    color: (0, 0, 0),
                    background: (0, 0, 0),
                    muted_color: (0, 0, 0),
                    muted: false,
                    idle: false,
                    scale: 1.0,
                };
            };
            let color = |text: &str| parse_color(text).unwrap_or((0, 0, 0));
            let playing = self.streams.values().any(|(s, c)| *c == Some(channel) && !s.corked);
            KnobLook {
                level: self.shown[channel],
                color: color(&config.color),
                background: color(&config.background),
                muted_color: color(&config.muted_color),
                muted: self.muted[channel],
                idle: config.controls_audio() && !playing,
                scale: if self.auto_lower().dim_leds && self.auto_lower().lowers(channel) {
                    self.ducker.factor().powf(2.2).max(0.08)
                } else {
                    1.0
                },
            }
        });
        let gamma = self.calibrating.map_or(self.config.leds.gamma, |(_, gamma)| gamma);
        let frame = if self.calibrating.is_some() {
            let colours = std::array::from_fn(|k| looks[k].color);
            led_frame(&calibration(&colours, self.config.leds.brightness, gamma))
        } else {
            let leds = &self.config.leds;
            led_frame(&colors(&looks, leds.brightness, leds.dim_idle, gamma, leds.blend))
        };
        let since = self.last_led_send.map(|t| now.duration_since(t));
        if self.last_frame == Some(frame) && since.is_some_and(|s| s < LED_REFRESH) {
            self.led_waiting = false;
            return None;
        }
        if since.is_some_and(|s| s < LED_EVERY) {
            self.led_waiting = true;
            return None;
        }
        self.led_waiting = false;
        self.last_frame = Some(frame);
        self.last_led_send = Some(now);
        Some(Out::Leds(frame))
    }

    /// Moves the LEDs' shown levels toward the real ones (exponential ease); first values and big gaps at startup
    /// snap. Sets `led_animating` while any knob is still gliding.
    fn ease_levels(&mut self, now: Instant) {
        // after a quiet spell, step as if one frame passed so a change still glides instead of jumping
        let dt = self.shown_at.map_or(0.0, |t| now.saturating_duration_since(t).min(LED_EVERY).as_secs_f64());
        self.shown_at = Some(now);
        let alpha = 1.0 - (-dt / LED_EASE).exp();
        let mut moving = false;
        for channel in 0..self.count() {
            let target = self.level(channel).map(f64::from);
            self.shown[channel] = match (self.shown[channel], target) {
                (Some(shown), Some(target)) => {
                    let next = shown + (target - shown) * alpha;
                    if (target - next).abs() <= 0.4 {
                        Some(target)
                    } else {
                        moving = true;
                        Some(next)
                    }
                }
                (_, target) => target,
            };
        }
        self.led_animating = moving;
    }

    fn target(&self, channel: usize) -> Option<u8> {
        let config = self.channels().get(channel)?;
        config.controls_audio().then_some(())?;
        let volume = config.volume_for(self.knob(channel)?);
        if self.auto_lower().lowers(channel) {
            return Some((f64::from(volume) * self.ducker.factor()).round() as u8);
        }
        Some(volume)
    }

    fn set_light(&mut self, entity: String, percent: u8) -> Vec<Out> {
        if let Some(state) = self.entities.get_mut(&entity) {
            state.brightness = Some(percent);
            state.state = if percent > 0 { "on" } else { "off" }.into();
        }
        vec![Out::Ha(Ha::Brightness { entity, percent })]
    }

    /// `turned`: the user moved the knob (startup sync and profile switches never move lights).
    fn apply(&mut self, channel: usize, turned: bool) -> Vec<Out> {
        if let Some(entity) = self.channels()[channel].light.clone() {
            let Some(knob) = self.knob(channel) else { return Vec::new() };
            let previous = self.applied[channel].replace(knob);
            let settling = self.lights_settle_until.zip(self.now).is_some_and(|(until, now)| now < until);
            if !turned || settling {
                return Vec::new();
            }
            // the first turn of a knob nobody has moved yet (say on screen without a mixer) sets the light outright
            let Some(previous) = previous else {
                return self.set_light(entity, knob);
            };
            if previous == knob {
                return Vec::new();
            }
            let state = self.entities.get_mut(&entity);
            let light = state.as_ref().and_then(|s| s.brightness).unwrap_or(previous);
            // a light that's off needs a deliberate turn up; until then keep counting from where the knob was
            if light == 0 && knob > previous && knob - previous < LIGHT_ON_MOVE {
                self.applied[channel] = Some(previous);
                return Vec::new();
            }
            let percent = catch_up(light, previous, knob);
            if self.verbose {
                eprintln!("light {entity}: {light}% -> {percent}% (knob {previous} -> {knob})");
            }
            return self.set_light(entity, percent);
        }
        let Some(percent) = self.target(channel) else {
            return Vec::new();
        };
        if self.verbose {
            eprintln!("channel {channel} -> {percent}%");
        }
        let mut out: Vec<Out> = self
            .streams
            .values()
            .filter(|(stream, c)| *c == Some(channel) && stream.writable)
            .map(|(stream, _)| Out::Audio(Audio::SetStream { index: stream.index, channels: stream.channels, percent }))
            .collect();
        if self.channels()[channel].master {
            out.push(Out::Audio(Audio::SetMaster { percent }));
        }
        out
    }

    /// New streams get their channel's volume (and mute); streams that changed their own volume get put back.
    fn guard(&self, stream: &Stream, channel: usize) -> Vec<Out> {
        let mut out = Vec::new();
        if self.muted[channel] && !stream.muted {
            out.push(Out::Audio(Audio::MuteStream { index: stream.index, mute: true }));
        }
        let Some(percent) = self.target(channel) else { return out };
        if stream.writable && (stream.volume - f64::from(percent)).abs() > GUARD_TOLERANCE {
            if self.verbose && self.streams.contains_key(&stream.index) {
                eprintln!("guard: stream {} was {:.0}%, back to {percent}%", stream.index, stream.volume);
            }
            out.push(Out::Audio(Audio::SetStream { index: stream.index, channels: stream.channels, percent }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;
    use cranked_protocol::{button_packet, knob_packet};

    fn channels() -> Vec<Channel> {
        let named = |app: &str| Channel { apps: vec![app.into()], ..Channel::default() };
        vec![
            Channel { press: Action::PlayPause, ..named("Firefox") },
            Channel { press: Action::NextProfile, hold: Action::None, ..named("Spotify") },
            Channel { min: 20, max: 60, ..named("Vesktop") },
            Channel { catch_all: true, press: Action::SwitchOutput, ..Channel::default() },
            Channel {
                light: Some("light.desk_lamp".into()),
                master: true,
                press: Action::HaToggle("switch.fan".into()),
                hold: Action::HaOff("light.desk_lamp".into()),
                ..Channel::default()
            },
        ]
    }

    fn config() -> Config {
        let mut second = channels();
        second[0].apps = vec!["Spotify".into()];
        second[1].apps = vec!["Firefox".into()];
        Config {
            popups: true,
            popup_style: Default::default(),
            theme: "cranked".into(),
            look: Default::default(),
            devices: Vec::new(),
            leds: Default::default(),
            home_assistant: Default::default(),
            profiles: vec![
                Profile {
                    name: "Main".into(),
                    channels: channels(),
                    auto_lower: AutoLower {
                        enabled: true,
                        lower: vec![1, 2],
                        triggers: vec![2, 3],
                        ..AutoLower::default()
                    },
                },
                Profile { name: "Swapped".into(), channels: second, auto_lower: AutoLower::default() },
            ],
        }
    }

    fn core() -> Core {
        Core::new(config(), &State::default())
    }

    fn stream(index: u32, app: &str, volume: f64) -> Stream {
        Stream {
            index,
            app_name: app.into(),
            binary: app.to_lowercase(),
            volume,
            channels: 2,
            writable: true,
            ..Stream::default()
        }
    }

    fn packets(knobs: &[(u8, u8)]) -> Event {
        Event::Bytes(knobs.iter().flat_map(|&(c, p)| knob_packet(c, p)).collect())
    }

    fn set_streams(out: &[Out]) -> Vec<(u32, u8)> {
        let mut found: Vec<_> = out
            .iter()
            .filter_map(|o| match o {
                Out::Audio(Audio::SetStream { index, percent, .. }) => Some((*index, *percent)),
                _ => None,
            })
            .collect();
        found.sort();
        found
    }

    fn popups(out: &[Out]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                Out::Desktop(Desktop::Text { text, .. }) => Some(text.clone()),
                Out::Desktop(Desktop::Volume { percent, name, .. }) => Some(format!("{name} {percent}%")),
                _ => None,
            })
            .collect()
    }

    fn synced(now: Instant) -> Core {
        let mut core = core();
        core.handle(Event::MixerConnected, now);
        core.handle(packets(&[(0, 50), (1, 40), (2, 50), (3, 70), (4, 30)]), now);
        core.tick(now + SYNC_QUIET);
        core
    }

    fn press(core: &mut Core, channel: u8, at: Instant) -> Vec<Out> {
        let mut out = core.handle(Event::Bytes(button_packet(channel, true).to_vec()), at);
        out.extend(core.handle(Event::Bytes(button_packet(channel, false).to_vec()), at + Duration::from_millis(100)));
        out
    }

    #[test]
    fn startup_sync_waits_for_the_warm_up_ramp_then_applies_once_without_popups() {
        let start = Instant::now();
        let mut core = core();
        core.handle(Event::Stream(stream(7, "Firefox", 100.0)), start);
        core.handle(Event::MixerConnected, start);
        for step in [10u8, 30, 50] {
            core.handle(packets(&[(0, step), (1, step), (2, step), (3, step), (4, step)]), start);
            assert!(set_streams(&core.tick(start + Duration::from_millis(100))).is_empty(), "nothing during the ramp");
        }
        let at = start + Duration::from_millis(100);
        core.handle(packets(&[(0, 50)]), at);
        assert!(set_streams(&core.tick(at + Duration::from_millis(100))).is_empty(), "not quiet yet");
        let out = core.tick(at + SYNC_QUIET);
        assert!(!core.is_syncing());
        assert_eq!(set_streams(&out), [(7, 50)]);
        assert!(popups(&out).is_empty());
    }

    #[test]
    fn sync_gives_up_waiting_for_silent_knobs() {
        let start = Instant::now();
        let mut core = core();
        core.handle(Event::MixerConnected, start);
        core.handle(packets(&[(0, 50)]), start);
        core.tick(start + SYNC_QUIET);
        assert!(core.is_syncing());
        core.tick(start + SYNC_LIMIT);
        assert!(!core.is_syncing());
        assert_eq!(core.knob(0), Some(50));
        assert_eq!(core.knob(1), None);
    }

    #[test]
    fn knob_turns_set_matching_streams_respect_the_range_and_pop_up() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Spotify", 40.0)), start);
        core.handle(Event::Stream(stream(2, "vesktop", 50.0)), start);
        let at = start + Duration::from_secs(1);
        core.handle(packets(&[(1, 45), (2, 100), (2, 100)]), at);
        let out = core.tick(at);
        assert_eq!(set_streams(&out), [(1, 45), (2, 60)]);
        assert_eq!(popups(&out), ["Knob 2 45%", "Knob 3 60%"]);
    }

    #[test]
    fn changes_are_coalesced_newest_wins() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Spotify", 40.0)), start);
        let at = start + Duration::from_secs(1);
        core.handle(packets(&[(1, 42)]), at);
        assert_eq!(set_streams(&core.tick(at)), [(1, 42)]);
        core.handle(packets(&[(1, 43), (1, 44), (1, 45)]), at);
        assert!(set_streams(&core.tick(at + Duration::from_millis(10))).is_empty());
        assert_eq!(set_streams(&core.tick(at + APPLY_EVERY)), [(1, 45)]);
    }

    #[test]
    fn new_streams_and_self_changed_streams_are_put_back() {
        let start = Instant::now();
        let mut core = synced(start);
        assert_eq!(set_streams(&core.handle(Event::Stream(stream(3, "Spotify", 100.0)), start)), [(3, 40)]);
        // our own write coming back changes nothing (no feedback loop)
        assert!(core.handle(Event::Stream(stream(3, "Spotify", 40.4)), start).is_empty());
        // Spotify rewriting its own volume on a seek
        assert_eq!(set_streams(&core.handle(Event::Stream(stream(3, "Spotify", 25.0)), start)), [(3, 40)]);
    }

    #[test]
    fn catch_all_takes_unknown_apps_and_lights_never_touch_audio() {
        let start = Instant::now();
        let mut core = synced(start);
        assert_eq!(set_streams(&core.handle(Event::Stream(stream(4, "Some Game", 100.0)), start)), [(4, 70)]);
        let at = start + Duration::from_secs(1);
        core.handle(packets(&[(4, 90), (4, 90)]), at);
        let out = core.tick(at);
        assert!(!out.iter().any(|o| matches!(o, Out::Audio(_))));
        assert_eq!(popups(&out), ["Knob 5 90%"]);
    }

    fn lamp(state: &str, brightness: Option<u8>) -> Event {
        Event::HaState(EntityState {
            entity: "light.desk_lamp".into(),
            name: "Desk Lamp".into(),
            state: state.into(),
            brightness,
        })
    }

    fn brightness(out: &[Out]) -> Vec<u8> {
        out.iter()
            .filter_map(|o| match o {
                Out::Ha(Ha::Brightness { percent, .. }) => Some(*percent),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn catch_up_math() {
        assert_eq!(catch_up(50, 50, 60), 60, "in step: 1:1");
        assert_eq!(catch_up(52, 50, 60), 60, "within 3: 1:1");
        // light at 80 elsewhere, knob at 40: turning up 40 -> 70 covers half of what's left
        assert_eq!(catch_up(80, 40, 70), 90);
        // light at 20, knob at 60: turning down 60 -> 30 halves it
        assert_eq!(catch_up(20, 60, 30), 10);
        assert_eq!(catch_up(80, 40, 0), 0);
        assert_eq!(catch_up(10, 60, 100), 100);
        assert_eq!(catch_up(37, 60, 60), 37);
    }

    #[test]
    fn lamp_knob_catches_up_and_never_moves_the_lamp_on_its_own() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(lamp("on", Some(80)), start);
        assert!(brightness(&core.tick(start + Duration::from_secs(1))).is_empty(), "sync never moves the lamp");
        // knob at 30, lamp at 80: turning up to 65 goes half of the remaining way
        let at = start + LIGHTS_SETTLE + Duration::from_secs(1);
        core.handle(packets(&[(4, 65), (4, 65)]), at);
        let out = core.tick(at);
        assert_eq!(brightness(&out), [90]);
        assert_eq!(popups(&out), ["Knob 5 90%"]);
        core.handle(packets(&[(4, 75)]), at + Duration::from_secs(1));
        assert_eq!(brightness(&core.tick(at + Duration::from_secs(1))), [93]);
        core.handle(packets(&[(4, 0), (4, 0)]), at + Duration::from_secs(2));
        assert_eq!(brightness(&core.tick(at + Duration::from_secs(2))), [0]);
    }

    #[test]
    fn ha_buttons_say_what_they_do_then_the_result() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(lamp("on", Some(40)), start);
        let at = start + Duration::from_secs(1);
        core.handle(Event::Bytes(button_packet(4, true).to_vec()), at);
        let out = core.tick(at + Duration::from_millis(600));
        assert_eq!(popups(&out), ["Turning off Desk Lamp"]);
        assert!(out.contains(&Out::Ha(Ha::Call { service: "turn_off", entity: "light.desk_lamp".into() })));
        assert_eq!(popups(&core.handle(lamp("off", Some(0)), at + Duration::from_secs(1))), ["Desk Lamp off"]);
        assert!(popups(&core.handle(lamp("on", Some(50)), at + Duration::from_secs(2))).is_empty(), "only once");
        assert_eq!(popups(&core.handle(Event::HaFailed, at)), ["Home Assistant didn't respond"]);
    }

    #[test]
    fn on_screen_input_skips_sync_and_spike_filter() {
        let start = Instant::now();
        let mut core = core();
        core.handle(Event::Stream(stream(1, "Spotify", 50.0)), start);
        core.handle(Event::OnScreen(knob_packet(1, 10).to_vec()), start);
        core.handle(Event::OnScreen(knob_packet(1, 90).to_vec()), start);
        assert_eq!(set_streams(&core.tick(start)), [(1, 90)]);
    }

    #[test]
    fn nothing_is_touched_before_the_knobs_are_known() {
        let start = Instant::now();
        let mut core = core();
        core.handle(Event::MixerConnected, start);
        assert!(core.handle(Event::Stream(stream(1, "Spotify", 100.0)), start).is_empty());
    }

    #[test]
    fn hold_mutes_the_channel_and_its_new_streams() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Firefox", 50.0)), start);
        let at = start + Duration::from_secs(1);
        core.handle(Event::Bytes(button_packet(0, true).to_vec()), at);
        let out = core.tick(at + Duration::from_millis(600));
        assert!(out.contains(&Out::Audio(Audio::MuteStream { index: 1, mute: true })));
        assert_eq!(popups(&out), ["Knob 1 muted"]);
        assert!(core.handle(Event::Bytes(button_packet(0, false).to_vec()), at + Duration::from_secs(1)).is_empty());
        let new = core.handle(Event::Stream(stream(2, "Firefox", 50.0)), at);
        assert!(new.contains(&Out::Audio(Audio::MuteStream { index: 2, mute: true })));
    }

    #[test]
    fn presses_run_their_actions() {
        let start = Instant::now();
        let mut core = synced(start);
        let at = start + Duration::from_secs(1);
        assert_eq!(press(&mut core, 0, at), [Out::Desktop(Desktop::Media(MediaKey::PlayPause))]);
        assert_eq!(press(&mut core, 3, at + Duration::from_secs(1)), [Out::Audio(Audio::SwitchOutput)]);
        assert_eq!(
            press(&mut core, 4, at + Duration::from_secs(2)),
            [
                Out::Desktop(Desktop::Text {
                    icon: "home".into(),
                    text: "Toggling switch.fan".into(),
                    style: Default::default()
                }),
                Out::Ha(Ha::Call { service: "toggle", entity: "switch.fan".into() })
            ]
        );
        let out = core.handle(Event::OutputSwitched(Some("LG Speakers".into())), at);
        assert_eq!(popups(&out), ["LG Speakers"]);
    }

    #[test]
    fn next_profile_moves_streams_and_remembers_it() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Firefox", 50.0)), start);
        core.handle(Event::Stream(stream(2, "Spotify", 40.0)), start);
        let at = start + Duration::from_secs(1);
        let out = press(&mut core, 1, at);
        assert!(out.iter().any(|o| matches!(o, Out::SaveState(s) if s.profile == "Swapped")));
        assert_eq!(popups(&out), ["Profile: Swapped"]);
        // Firefox now on knob 2 (40%), Spotify on knob 1 (50%)
        assert_eq!(set_streams(&core.tick(at + Duration::from_millis(200))), [(1, 40), (2, 50)]);
        let restarted = Core::new(config(), &State { profile: "Swapped".into(), ..State::default() });
        assert_eq!(restarted.profile, 1);
    }

    #[test]
    fn leds_follow_levels_and_are_throttled() {
        let start = Instant::now();
        let frames = |out: &[Out]| out.iter().filter(|o| matches!(o, Out::Leds(_))).count();
        let mut core = core();
        core.handle(Event::MixerConnected, start);
        core.handle(packets(&[(0, 50), (1, 40), (2, 50), (3, 70), (4, 30)]), start);
        assert_eq!(frames(&core.tick(start)), 0, "dark during the startup sync");
        assert_eq!(frames(&core.tick(start + SYNC_QUIET)), 1, "lit when it ends");
        let at = start + Duration::from_secs(1);
        assert_eq!(frames(&core.tick(start + SYNC_QUIET + Duration::from_millis(100))), 0, "unchanged");
        assert_eq!(frames(&core.tick(start + SYNC_QUIET + LED_REFRESH)), 1, "refreshed");
        assert_eq!(core.next_wake(), LED_REFRESH);
        core.handle(packets(&[(0, 90), (0, 90)]), at);
        assert_eq!(frames(&core.tick(at)), 1, "a change shows right away");
        // a big move, so the LEDs really change (small moves inside one LED's third don't)
        core.handle(packets(&[(0, 10), (0, 10)]), at);
        assert_eq!(frames(&core.tick(at + LED_EVERY / 2)), 0, "but at most 25 frames a second");
        assert!(core.next_wake() <= LED_EVERY);
        assert_eq!(frames(&core.tick(at + LED_EVERY)), 1);
        core.handle(Event::MixerLost, at);
        core.handle(packets(&[(0, 10)]), at);
        assert_eq!(frames(&core.tick(at + Duration::from_secs(1))), 0, "no mixer, no LEDs");
    }

    #[test]
    fn leds_glide_to_a_new_level_then_settle() {
        let start = Instant::now();
        let mut core = synced(start);
        let at = start + Duration::from_secs(1);
        core.handle(packets(&[(0, 100), (0, 100)]), at);
        core.tick(at);
        let first = core.shown[0].unwrap();
        assert!(first < 99.0, "doesn't jump straight there: {first}");
        let mut t = at;
        for _ in 0..4 {
            t += LED_EVERY;
            core.tick(t);
        }
        let later = core.shown[0].unwrap();
        assert!(later > first && later < 100.0, "still gliding: {first} -> {later}");
        assert_eq!(core.next_wake(), LED_EVERY, "animating at the frame rate");
        for _ in 0..10 {
            t += LED_EVERY;
            core.tick(t);
        }
        assert_eq!(core.shown[0], Some(100.0), "settles exactly, in well under half a second");
        assert_eq!(core.next_wake(), LED_REFRESH, "back to idle pacing");
    }

    fn meters(out: &[Out]) -> Vec<(u32, bool)> {
        out.iter()
            .filter_map(|o| match o {
                Out::Audio(Audio::Meter { index, on, .. }) => Some((*index, *on)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_triggers_and_voice_chat_are_metered_never_lowered_knobs() {
        let start = Instant::now();
        let mut core = synced(start);
        // knob 3 (Vesktop) triggers; knob 2 (Spotify) triggers but is also lowered, so it doesn't
        assert_eq!(meters(&core.handle(Event::Stream(stream(1, "vesktop", 50.0)), start)), [(1, true)]);
        assert!(meters(&core.handle(Event::Stream(stream(2, "Spotify", 40.0)), start)).is_empty());
        assert!(meters(&core.handle(Event::Stream(stream(3, "Firefox", 50.0)), start)).is_empty());
        let mut call = stream(4, "Some Game", 70.0);
        call.media_role = "phone".into();
        assert_eq!(meters(&core.handle(Event::Stream(call), start)), [(4, true)], "voice chat anywhere counts");
        assert!(meters(&core.handle(Event::Stream(stream(1, "vesktop", 51.0)), start)).is_empty(), "no churn");
        assert_eq!(meters(&core.handle(Event::StreamRemoved(1), start)), [(1, false)]);
        // a profile without Auto-lower stops all meters
        let out = press(&mut core, 1, start + Duration::from_secs(1));
        assert_eq!(meters(&out), [(4, false)]);
    }

    #[test]
    fn talking_lowers_the_lowered_knobs_and_their_new_streams_then_restores() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "vesktop", 50.0)), start);
        core.handle(Event::Stream(stream(2, "Firefox", 50.0)), start);
        core.handle(Event::Stream(stream(3, "Spotify", 40.0)), start);
        let mut now = start + Duration::from_secs(1);
        core.tick(now);
        // talking: loud reports every 200 ms (a single one, like a notification ping, lowers nothing)
        core.handle(Event::Loud(1), now);
        assert!(set_streams(&core.tick(now + Duration::from_millis(100))).is_empty(), "not on the first sound");
        for _ in 0..3 {
            now += Duration::from_millis(200);
            core.tick(now);
            core.handle(Event::Loud(1), now);
        }
        let mut last = Vec::new();
        for _ in 0..8 {
            now += Duration::from_millis(50);
            let out = core.tick(now);
            if !set_streams(&out).is_empty() {
                last = set_streams(&out);
            }
        }
        // 30% down: Firefox 50 -> 35, Spotify 40 -> 28; Vesktop (the trigger) untouched
        assert_eq!(last, [(2, 35), (3, 28)]);
        assert_eq!(set_streams(&core.handle(Event::Stream(stream(5, "Firefox", 100.0)), now)), [(5, 35)]);
        assert!(set_streams(&core.handle(Event::Stream(stream(2, "Firefox", 35.0)), now)).is_empty(), "guard agrees");
        assert!(core.next_wake() <= APPLY_EVERY);
        for _ in 0..60 {
            now += Duration::from_millis(50);
            let out = core.tick(now);
            if !set_streams(&out).is_empty() {
                last = set_streams(&out);
            }
        }
        assert_eq!(last, [(2, 50), (3, 40), (5, 50)]);
        assert_eq!(core.next_wake(), LED_REFRESH, "back to idle pacing");
    }

    #[test]
    fn a_lowered_knob_cannot_trigger_itself() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(2, "Firefox", 50.0)), start);
        core.handle(Event::Loud(2), start);
        assert!(!core.ducking);
    }

    #[test]
    fn snapshot_shows_what_each_knob_does_now() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Spotify", 40.0)), start);
        core.handle(lamp("on", Some(35)), start);
        let snapshot = core.snapshot();
        assert!(snapshot.mixer);
        assert_eq!(snapshot.profile, "Main");
        assert_eq!(snapshot.knobs[1].source, "Spotify");
        assert_eq!(snapshot.knobs[1].level, Some(40));
        assert_eq!(snapshot.knobs[1].press, "Next profile");
        assert_eq!(snapshot.knobs[0].source, "Nothing playing");
        assert_eq!(snapshot.knobs[4].source, "Desk Lamp 35%");
        assert_eq!(snapshot.knobs[4].level, Some(35));
        assert_eq!(snapshot.knobs[4].hold, "Turn off Desk Lamp");
        assert_eq!(snapshot.knobs[3].source, "Nothing playing");
    }

    #[test]
    fn new_settings_rematch_streams_without_moving_lights() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Spotify", 40.0)), start);
        core.handle(lamp("on", Some(35)), start);
        let mut config = config();
        config.profiles[0].channels[0].apps = vec!["Spotify".into()];
        config.profiles[0].channels[1].apps.clear();
        let mut out = Vec::new();
        core.set_config(config, &mut out);
        let out = core.tick(start + Duration::from_secs(1));
        assert_eq!(set_streams(&out), [(1, 50)], "Spotify now follows knob 1");
        assert!(brightness(&out).is_empty());
    }

    #[test]
    fn without_a_mixer_knobs_start_where_they_were_and_lights_stay_put() {
        let start = Instant::now();
        let mut state = State::default();
        state.knobs.insert("2".into(), 40);
        state.knobs.insert("5".into(), 90);
        let mut core = Core::starting_at(config(), &state, start);
        core.handle(Event::Stream(stream(1, "Spotify", 100.0)), start);
        core.handle(lamp("on", Some(20)), start);
        assert!(set_streams(&core.tick(start + Duration::from_secs(1))).is_empty(), "waits for a mixer first");
        let out = core.tick(start + NO_MIXER_AFTER);
        assert_eq!(set_streams(&out), [(1, 40)]);
        assert!(brightness(&out).is_empty());
        assert_eq!(core.knob(0), None, "never-saved knobs stay unknown");
    }

    #[test]
    fn a_mixer_always_wins_over_saved_positions() {
        let start = Instant::now();
        let mut state = State::default();
        state.knobs.insert("2".into(), 40);
        let mut core = Core::starting_at(config(), &state, start);
        core.handle(Event::MixerConnected, start);
        core.handle(packets(&[(0, 50), (1, 70), (2, 50), (3, 70), (4, 30)]), start);
        core.tick(start + SYNC_QUIET);
        core.tick(start + NO_MIXER_AFTER);
        assert_eq!(core.knob(1), Some(70));
    }

    #[test]
    fn knobs_past_the_mixer_start_where_they_were() {
        let start = Instant::now();
        let mut config = config();
        for name in ["Keyboard", "Extra"] {
            config.profiles[0].channels.push(Channel { name: name.into(), ..Channel::default() });
        }
        let mut state = State::default();
        state.knobs.insert("2".into(), 40);
        state.knobs.insert("6".into(), 60);
        let mut core = Core::starting_at(config, &state, start);
        core.handle(Event::MixerConnected, start);
        core.handle(packets(&[(0, 50), (1, 70), (2, 50), (3, 70), (4, 30)]), start);
        core.tick(start + SYNC_QUIET);
        assert_eq!(core.knob(1), Some(70), "the mixer's own knobs follow the mixer");
        assert_eq!(core.knob(5), Some(60), "knob 6 remembered");
        assert_eq!(core.knob(6), None, "never saved: still unknown");
    }

    #[test]
    fn positions_are_saved_once_still() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::OnScreen(knob_packet(1, 66).to_vec()), Instant::now());
        let saved = |out: &[Out]| {
            out.iter().find_map(|o| match o {
                Out::SaveState(s) => Some(s.knobs.get("2").copied()),
                _ => None,
            })
        };
        assert_eq!(saved(&core.tick(Instant::now())), None, "not while moving");
        assert_eq!(saved(&core.tick(Instant::now() + SAVE_AFTER)), Some(Some(66)));
    }

    #[test]
    fn leds_go_dark_with_the_screen_and_wake_on_touch() {
        let start = Instant::now();
        let mut core = synced(start);
        let frames = |out: &[Out]| {
            out.iter()
                .filter_map(|o| match o {
                    Out::Leds(f) => Some(*f),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let at = start + Duration::from_secs(1);
        core.handle(Event::ScreenOff(true), at);
        let dark = frames(&core.tick(at));
        assert_eq!(dark.len(), 1);
        assert!(dark[0][2..47].iter().all(|b| *b == 0), "one all-dark frame");
        assert!(frames(&core.tick(at + Duration::from_secs(1))).is_empty(), "then nothing: no refresh while dark");
        assert_eq!(core.next_wake(), Duration::from_secs(1), "no 4-a-second wakeups overnight");
        let touch = at + Duration::from_secs(2);
        core.handle(packets(&[(0, 60)]), touch);
        let lit = frames(&core.tick(touch));
        assert!(lit.len() == 1 && lit[0][2..47].iter().any(|b| *b != 0), "touching the mixer lights it");
        assert_eq!(frames(&core.tick(touch + WAKE_FOR)).len(), 1, "dark again after a while");
        core.handle(Event::ScreenOff(false), touch + WAKE_FOR);
        assert!(frames(&core.tick(touch + WAKE_FOR + Duration::from_millis(300)))[0][2..47].iter().any(|b| *b != 0));
    }

    #[test]
    fn remembers_apps_it_has_seen_once_each() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(Event::Stream(stream(1, "Spotify", 40.0)), start);
        core.handle(Event::Stream(stream(2, "spotify", 40.0)), start);
        core.handle(Event::Stream(stream(3, "Firefox", 40.0)), start);
        core.handle(Event::Stream(stream(4, "nvbroadcast", 40.0)), start);
        core.handle(Event::StreamRemoved(3), start);
        assert_eq!(core.snapshot().known_apps, ["Firefox", "spotify"]);
        assert_eq!(core.state().apps, ["spotify", "Firefox"]);
    }

    #[test]
    fn calibration_shows_each_colour_full_then_dimmed_and_ends_by_itself() {
        let start = Instant::now();
        let mut core = synced(start);
        let frame = |out: &[Out]| {
            out.iter().find_map(|o| match o {
                Out::Leds(f) => Some(*f),
                _ => None,
            })
        };
        let at = start + Duration::from_secs(1);
        core.calibrate(Some([1.0, 1.0, 1.0]), at);
        let shown = frame(&core.tick(at)).expect("the pattern goes out right away");
        // knob 1 (default colour #00C8FF at 80% brightness): full on the left, dimmer to the right
        let (left, middle, right) = (shown[3], shown[6], shown[9]);
        assert!(left > middle && middle > right, "{left} {middle} {right}");
        core.calibrate(Some([1.0, 9.0, 1.0]), at);
        let corrected = frame(&core.tick(at + Duration::from_millis(100))).unwrap();
        assert!(corrected[6] > shown[6], "more green kept when dimmed");
        assert!(core.calibrating.is_some());
        core.tick(at + CALIBRATION_TIMEOUT);
        assert!(core.calibrating.is_none(), "ends by itself");
    }

    #[test]
    fn lights_ignore_the_mixers_warm_up_and_never_switch_on_from_a_wobble() {
        let start = Instant::now();
        let mut core = synced(start);
        core.handle(lamp("off", Some(0)), start);
        // right after connecting, a reading moves the lamp knob: nothing is sent
        let soon = start + Duration::from_secs(1);
        core.handle(packets(&[(4, 34)]), soon);
        assert!(brightness(&core.tick(soon)).is_empty(), "settling after the mixer connects");
        // later, a 1-2% wobble on a lamp that's off never switches it on
        let later = start + Duration::from_secs(10);
        core.handle(packets(&[(4, 35)]), later);
        assert!(brightness(&core.tick(later)).is_empty());
        // but turning it up on purpose (even slowly) does
        core.handle(packets(&[(4, 37)]), later + Duration::from_secs(1));
        assert!(!brightness(&core.tick(later + Duration::from_secs(1))).is_empty(), "34 -> 37 counts as a turn");
    }

    fn sized(count: usize) -> Config {
        let named = |i: usize| Channel { apps: vec![format!("App{i}")], ..Channel::default() };
        Config {
            profiles: vec![Profile {
                name: "P".into(),
                channels: (0..count).map(named).collect(),
                auto_lower: Default::default(),
            }],
            ..config()
        }
    }

    #[test]
    fn profiles_can_have_more_knobs_than_the_mixer() {
        let start = Instant::now();
        let mut core = Core::new(sized(7), &State::default());
        core.handle(Event::Stream(stream(1, "App6", 50.0)), start);
        core.handle(Event::OnScreen(knob_packet(6, 30).to_vec()), start);
        assert_eq!(set_streams(&core.tick(start)), [(1, 30)], "knob 7 runs its app");
        assert_eq!(core.snapshot().knobs.len(), 7);
        // a button on knob 11 (which doesn't exist) does nothing
        core.handle(Event::OnScreen(button_packet(10, true).to_vec()), start);
        assert!(core
            .handle(Event::OnScreen(button_packet(10, false).to_vec()), start + Duration::from_millis(100))
            .is_empty());
    }

    #[test]
    fn fewer_knobs_than_the_mixer_leaves_the_rest_dark_and_idle() {
        let start = Instant::now();
        let mut core = Core::new(sized(3), &State::default());
        core.handle(Event::Stream(stream(1, "App2", 50.0)), start);
        core.handle(Event::MixerConnected, start);
        core.handle(packets(&[(0, 50), (1, 40), (2, 60), (3, 70), (4, 30)]), start);
        let out = core.tick(start + SYNC_QUIET);
        assert_eq!(set_streams(&out), [(1, 60)]);
        let frame = out
            .iter()
            .find_map(|o| match o {
                Out::Leds(f) => Some(*f),
                _ => None,
            })
            .unwrap();
        // knobs 4 and 5 (bytes for LEDs 9..15) stay dark
        assert!(frame[2 + 9 * 3..2 + 15 * 3].iter().all(|b| *b == 0));
        assert!(frame[2..2 + 9 * 3].iter().any(|b| *b != 0));
        // turning the mixer's knob 5 later does nothing
        let later = start + Duration::from_secs(10);
        core.handle(packets(&[(4, 90), (4, 90)]), later);
        assert!(set_streams(&core.tick(later)).is_empty());
    }
}
