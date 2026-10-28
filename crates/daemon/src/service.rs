//! The running service: starts the worker threads, then runs the event loop. Each event is routed (windows,
//! tray, D-Bus, devices and updates are answered here; everything else goes to `Core`), and what `Core` decides is
//! sent on to the threads that act on it.

use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use cranked_ipc::settings::MAX_CONTROLS;
use cranked_ipc::{ServerMessage, UnixListener};
use cranked_protocol::convert::saturate_u8;
use cranked_protocol::{button_packet, knob_packet, led_frame};

use crate::config::{Config, State};
use crate::core::{Core, Out};
use crate::platform::Platform;
use crate::{
    audio, config, dbus, desktop, devices, home_assistant, parse_line, serial, server, shortcuts, tray, updater, Event,
    Options,
};

/// How often the settings file is checked for changes made in an editor.
const CONFIG_CHECK_EVERY: Duration = Duration::from_secs(1);

/// What handling one event asked for besides `Core::handle`; applied once the event is through.
#[derive(Default)]
struct Requests {
    reload: bool,
    new_config: Option<Config>,
    switch_to: Option<String>,
    cycle_profile: bool,
    restart_home_assistant: bool,
}

pub struct Service {
    core: Core,
    events: Sender<Event>,
    /// This platform's sound system (it runs its own thread; the service lists outputs through it).
    audio_system: Box<dyn audio::AudioSystem>,
    audio: Sender<audio::Command>,
    desktop: Sender<desktop::Command>,
    leds: serial::Writer,
    clients: server::Clients,
    remote_api: Box<dyn dbus::RemoteApi>,
    tray: Box<dyn tray::TrayIcon>,
    /// What the tray last showed: profile, profiles, mixer connected.
    tray_shows: (String, Vec<String>, bool),
    shortcuts: Box<dyn shortcuts::GlobalShortcuts>,
    home_assistant_calls: Option<Sender<home_assistant::Command>>,
    home_assistant_generation: Arc<AtomicU64>,
    /// This platform's keyboard knobs and serial ports; the configured devices run as `devices::InputSource`s.
    input_hardware: Box<dyn devices::InputHardware>,
    device_generation: Arc<AtomicU64>,
    /// This platform's way of installing updates.
    updater: Box<dyn updater::Updater>,
    latest_release: Option<updater::ReleaseInfo>,
    config_path: PathBuf,
    config_modified: Option<SystemTime>,
    config_checked: Instant,
    state_path: PathBuf,
    dry_run: bool,
}

impl Service {
    /// Starts every worker thread and the desktop integrations (D-Bus API, shortcuts, tray).
    pub fn start(
        platform: Platform,
        options: &Options,
        config: Config,
        listener: UnixListener,
        events: Sender<Event>,
    ) -> Self {
        let state_path = options.state_path.clone();
        let state = State::load(&state_path);
        let (audio, audio_inbox) = mpsc::channel();
        let (desktop, desktop_inbox) = mpsc::channel();
        let leds = serial::Writer::default();

        platform.desktop.start(desktop_inbox);
        platform.desktop.watch_screen(events.clone());
        platform.audio.start(audio_inbox, events.clone());
        if options.use_mixer {
            let (events, leds) = (events.clone(), leds.clone());
            spawn("mixer", move || serial::run(&events, &leds));
        }
        if options.use_stdin {
            let events = events.clone();
            spawn("stdin", move || read_stdin(&events));
        }
        {
            let events = events.clone();
            spawn("windows", move || server::run(&listener, &events));
        }

        let mut core = Core::new(config, &state);
        core.verbose = options.verbose;
        let device_generation = Arc::new(AtomicU64::new(0));
        let input_hardware = platform.input_hardware;
        devices::start(devices::sources(&core.config().devices, &*input_hardware), &events, &device_generation);
        let mut shortcuts = platform.shortcuts;
        shortcuts.start(events.clone(), most_knobs(core.config()), &core.config().shortcuts);
        let mut tray = platform.tray;
        tray.start(tray::Tray { events: events.clone(), profile: String::new(), profiles: Vec::new(), mixer: false });
        let mut remote_api = platform.remote_api;
        remote_api.start(events.clone());
        let config_path = options.config_path.clone();
        let mut service = Self {
            core,
            remote_api,
            events,
            audio_system: platform.audio,
            audio,
            desktop,
            leds,
            clients: server::Clients::default(),
            tray,
            tray_shows: (String::new(), Vec::new(), false),
            shortcuts,
            home_assistant_calls: None,
            home_assistant_generation: Arc::new(AtomicU64::new(0)),
            input_hardware,
            device_generation,
            updater: platform.updater,
            latest_release: None,
            config_modified: modified(&config_path),
            config_path,
            config_checked: Instant::now(),
            state_path,
            dry_run: options.dry_run,
        };
        service.restart_home_assistant();
        if options.open_window {
            spawn_detached(&sibling("cranked-audio"), &[]);
        }
        service
    }

    /// Runs until the audio thread or every event sender is gone.
    pub fn run(mut self, inbox: &Receiver<Event>) {
        loop {
            let event = match inbox.recv_timeout(self.core.next_wake()) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            let now = Instant::now();
            let mut requests = Requests::default();
            let event = event.and_then(|event| self.route(event, &mut requests));
            let mut out = event.map(|event| self.core.handle(event, now)).unwrap_or_default();
            self.apply(requests, now, &mut out);
            out.extend(self.core.tick(now));
            self.publish();
            if !self.dispatch(out) {
                return;
            }
        }
    }

    /// Answers the events the service handles itself; returns the rest (some turned into knob input) for `Core`.
    fn route(&mut self, event: Event, requests: &mut Requests) -> Option<Event> {
        match event {
            Event::ClientConnected(id, stream) => {
                self.clients.add(id, stream, &self.core.snapshot());
                None
            }
            Event::ClientDisconnected(id) => {
                self.clients.remove(id);
                None
            }
            Event::ClientLine(line) => {
                self.client_line(&line, requests);
                None
            }
            Event::Tray(action) => {
                self.tray_action(action, requests);
                None
            }
            Event::Remote(request) => self.remote(request, requests),
            Event::Device(input) => self.device(&input),
            Event::MidiLearned(learned) => {
                self.clients.send(&ServerMessage::MidiLearned(learned.map(|l| cranked_ipc::LearnedControl {
                    port: l.port,
                    channel: l.channel,
                    kind: l.kind,
                    number: l.number,
                })));
                None
            }
            Event::HomeAssistantEntities(entities) => {
                self.clients.send(&ServerMessage::Entities(entities));
                None
            }
            Event::UpdateCheckResult(result) => {
                self.update_checked(result);
                None
            }
            Event::UpdateApplyResult(result) => {
                self.update_applied(result);
                None
            }
            other => Some(other),
        }
    }

    /// One line from a window: a request (`get-config`, `config {...}`, `profile NAME`, ...) or on-screen input.
    fn client_line(&mut self, line: &str, requests: &mut Requests) {
        let (verb, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        match verb {
            "reload" => requests.reload = true,
            "get-config" => self.clients.send(&ServerMessage::Config(self.core.config().clone())),
            "entities" => match &self.home_assistant_calls {
                Some(calls) => {
                    let _ = calls.send(home_assistant::Command::ListEntities);
                }
                None => self.clients.send(&ServerMessage::Entities(Vec::new())),
            },
            "config" => match serde_json::from_str::<Config>(rest) {
                Ok(mut config) => {
                    config.normalize();
                    match config.save(&self.config_path) {
                        Ok(()) => {
                            self.config_modified = modified(&self.config_path);
                            requests.new_config = Some(config);
                        }
                        Err(e) => eprintln!("settings: can't save {} ({e})", self.config_path.display()),
                    }
                }
                Err(e) => eprintln!("settings from the window: {e}"),
            },
            "token" => {
                let path = self.core.config().token_path(&self.config_path);
                match save_secret(&path, rest.trim()) {
                    Ok(()) => requests.restart_home_assistant = true,
                    Err(e) => eprintln!("home assistant: can't save the token ({e})"),
                }
            }
            "profile" => requests.switch_to = Some(rest.to_string()),
            "list-devices" => {
                let (keyboards, typing) = self.input_hardware.keyboard_knobs();
                self.clients.send(&ServerMessage::Devices(cranked_ipc::DeviceChoices {
                    keyboards,
                    typing,
                    midi: devices::midi_controllers(),
                    serial: self.input_hardware.serial_ports(),
                    outputs: self.audio_system.outputs(),
                }));
            }
            "learn-midi" => {
                let events = self.events.clone();
                std::thread::spawn(move || devices::learn_midi(&events, Duration::from_secs(15)));
            }
            // `calibrate R G B` shows the LED calibration pattern with that correction; `calibrate off` ends it
            "calibrate" => {
                let values: Vec<f64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                let gamma = <[f64; 3]>::try_from(values).ok();
                self.core.calibrate(gamma, Instant::now());
            }
            "check-update" => self.updater.check(self.events.clone()),
            "apply-update" => {
                if !self.updater.can_install() {
                    let status = "This copy came from a package manager; update it there.".into();
                    self.update_status(status, None, false);
                } else if let Some(info) = self.latest_release.clone() {
                    self.updater.install(info, self.events.clone());
                } else {
                    self.update_status("No update found to install. Check for updates first.".into(), None, false);
                }
            }
            _ => match parse_line(line) {
                Some(bytes) => {
                    let _ = self.events.send(Event::OnScreen(bytes));
                }
                None => eprintln!("window: ? {line}"),
            },
        }
    }

    fn tray_action(&mut self, action: tray::Action, requests: &mut Requests) {
        match action {
            tray::Action::OpenWindow => self.open_window(),
            tray::Action::Profile(name) => requests.switch_to = Some(name),
            tray::Action::EditSettings => {
                if !self.config_path.exists() {
                    let _ = self.core.config().save(&self.config_path);
                }
                let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
                spawn_detached(Path::new(opener), &[self.config_path.as_os_str()]);
            }
            tray::Action::Quit => {
                eprintln!("quit from the tray");
                self.clients.send(&ServerMessage::Quit);
                self.leds.send(&led_frame(&[[(0, 0, 0); cranked_protocol::LEDS_PER_KNOB]; cranked_protocol::KNOBS]));
                self.leds.flush();
                let _ = std::fs::remove_file(cranked_ipc::socket_path());
                std::process::exit(0);
            }
        }
    }

    /// A D-Bus API request; knob and button requests become on-screen input for `Core`.
    fn remote(&mut self, request: dbus::Request, requests: &mut Requests) -> Option<Event> {
        match request {
            dbus::Request::SetKnob { knob, percent } => set_knob(knob, percent),
            dbus::Request::StepKnob { knob, delta } => self.step_knob(knob, delta),
            dbus::Request::Button { button, down } => press(button, down),
            dbus::Request::Profile(name) => {
                requests.switch_to = Some(name);
                None
            }
            dbus::Request::OpenWindow => {
                self.open_window();
                None
            }
            dbus::Request::NextProfile => {
                requests.cycle_profile = true;
                None
            }
            dbus::Request::OpenSettings => {
                if self.clients.is_empty() {
                    spawn_detached(&sibling("cranked-audio"), &["--page".as_ref(), "settings".as_ref()]);
                } else {
                    self.clients.send(&ServerMessage::OpenPage("settings".into()));
                }
                None
            }
        }
    }

    /// Input from a keyboard knob, MIDI controller or deej box; media keys go straight to the desktop.
    fn device(&self, input: &devices::Input) -> Option<Event> {
        match *input {
            devices::Input::Set { knob, percent } => set_knob(knob, percent),
            devices::Input::Step { knob, delta } => self.step_knob(knob, delta),
            devices::Input::Button { knob, down } => press(knob, down),
            devices::Input::Media(key) => {
                let _ = self.desktop.send(desktop::Command::Media(key));
                None
            }
        }
    }

    /// Turns `knob` by `delta` percent from where it is now (0 when it has no position yet).
    fn step_knob(&self, knob: usize, delta: i32) -> Option<Event> {
        let current = i32::from(self.core.knob(knob).unwrap_or(0));
        set_knob(knob, saturate_u8((current + delta).clamp(0, 100)))
    }

    /// Brings an open window to the front, or starts one.
    fn open_window(&mut self) {
        if self.clients.is_empty() {
            spawn_detached(&sibling("cranked-audio"), &[]);
        } else {
            self.clients.send(&ServerMessage::OpenPage(String::new()));
        }
    }

    fn update_checked(&mut self, result: Result<Option<updater::ReleaseInfo>, String>) {
        match result {
            Ok(Some(info)) => {
                let version = info.version.clone();
                self.latest_release = Some(info);
                if self.updater.can_install() {
                    self.update_status(format!("Update available: {version}"), Some(version), true);
                } else {
                    let status = format!("Update available: {version}. Update it with your package manager.");
                    self.update_status(status, Some(version), false);
                }
            }
            Ok(None) => {
                self.latest_release = None;
                self.update_status(format!("Up to date ({})", updater::CURRENT_VERSION), None, false);
            }
            Err(e) => self.update_status(format!("Check failed: {e}"), None, false),
        }
    }

    fn update_applied(&mut self, result: Result<(), String>) {
        let installed = result.is_ok();
        let status = match result {
            Ok(()) => "Update installed! Restarting...".into(),
            Err(e) => format!("Install failed: {e}"),
        };
        self.update_status(status, None, false);
        // Linux: install.sh restarts the service. Windows: this one starts the new one and leaves.
        if installed && self.updater.restarts_service() {
            self.restart_into_update();
        }
    }

    /// Windows: starts the new service, which waits for this one to exit (socket, mixer port) and reopens the
    /// window if one was open, with the same options this one had (minus the restart ones).
    fn restart_into_update(&mut self) {
        let mut kept: Vec<std::ffi::OsString> = Vec::new();
        let mut old = std::env::args_os().skip(1);
        while let Some(arg) = old.next() {
            match arg.to_str() {
                Some("--wait-pid") => {
                    old.next();
                }
                Some("--open-window") => {}
                _ => kept.push(arg),
            }
        }
        let pid = std::process::id().to_string();
        let mut args: Vec<&std::ffi::OsStr> = kept.iter().map(std::ffi::OsString::as_os_str).collect();
        args.extend([std::ffi::OsStr::new("--wait-pid"), std::ffi::OsStr::new(&pid)]);
        if !self.clients.is_empty() {
            args.push("--open-window".as_ref());
        }
        spawn_detached(&sibling("cranked-audiod"), &args);
        self.clients.send(&ServerMessage::Quit);
        let _ = std::fs::remove_file(cranked_ipc::socket_path());
        std::process::exit(0);
    }

    fn update_status(&mut self, status: String, latest_version: Option<String>, can_install: bool) {
        self.clients.send(&ServerMessage::UpdateStatus { status, latest_version, can_install });
    }

    /// Profile switches, settings changes (from a window or edited on disk) and Home Assistant restarts.
    fn apply(&mut self, mut requests: Requests, now: Instant, out: &mut Vec<Out>) {
        if let Some(name) = requests.switch_to {
            self.core.switch_profile(&name, out);
        }
        if requests.cycle_profile {
            self.core.cycle_profile(out);
        }
        if now.duration_since(self.config_checked) >= CONFIG_CHECK_EVERY {
            self.config_checked = now;
            let latest = modified(&self.config_path);
            requests.reload |= latest != self.config_modified;
            self.config_modified = latest;
        }
        if requests.reload {
            match Config::load(&self.config_path) {
                Ok(config) => requests.new_config = Some(config),
                Err(e) => eprintln!("settings: {e} (keeping the previous settings)"),
            }
        }
        if let Some(config) = requests.new_config {
            requests.restart_home_assistant |= self.set_config(config, out);
        }
        if requests.restart_home_assistant {
            self.restart_home_assistant();
        }
    }

    /// Applies new settings everywhere. Returns whether Home Assistant needs restarting.
    fn set_config(&mut self, config: Config, out: &mut Vec<Out>) -> bool {
        eprintln!("settings: applied");
        let current = self.core.config();
        let home_assistant_changed = config.home_assistant != current.home_assistant
            || config.home_assistant_entities() != current.home_assistant_entities();
        if config.devices != current.devices {
            let sources = devices::sources(&config.devices, &*self.input_hardware);
            devices::start(sources, &self.events, &self.device_generation);
        }
        self.shortcuts.ensure(most_knobs(&config), &config.shortcuts);
        self.core.set_config(config, out);
        self.clients.send(&ServerMessage::Config(self.core.config().clone()));
        home_assistant_changed
    }

    /// Stops the Home Assistant threads (if any) and starts new ones with the current settings.
    fn restart_home_assistant(&mut self) {
        self.home_assistant_calls = self.start_home_assistant();
        self.core.home_assistant_enabled = self.home_assistant_calls.is_some();
    }

    /// Starts the state watcher and the service-call thread; older ones stop once the generation moves past them.
    fn start_home_assistant(&self) -> Option<Sender<home_assistant::Command>> {
        let mine = self.home_assistant_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let settings = config::home_assistant_settings(self.core.config(), &self.config_path)?;
        let (calls, calls_inbox) = mpsc::channel();
        let (states, watched, generation) = (
            self.events.clone(),
            self.core.config().home_assistant_entities(),
            Arc::clone(&self.home_assistant_generation),
        );
        let for_states = settings.clone();
        spawn("home-assistant-states", move || {
            home_assistant::run_states(&for_states, &watched, &states, &generation, mine);
        });
        let failures = self.events.clone();
        spawn("home-assistant-calls", move || home_assistant::run_calls(&settings, &calls_inbox, &failures));
        Some(calls)
    }

    /// Sends the current state to the remote API's listeners, windows and the tray.
    fn publish(&mut self) {
        let snapshot = self.core.snapshot();
        self.remote_api.publish(&snapshot);
        if !self.clients.is_empty() {
            self.clients.update(&snapshot);
        }
        let shows = (snapshot.profile, snapshot.profiles, snapshot.mixer);
        if shows != self.tray_shows {
            self.tray_shows.clone_from(&shows);
            let (profile, profiles, mixer) = shows;
            self.tray.show(profile, profiles, mixer);
        }
    }

    /// Hands `Core`'s decisions to the threads that act on them. False when the audio thread is gone.
    fn dispatch(&self, out: Vec<Out>) -> bool {
        if self.dry_run {
            return true;
        }
        for item in out {
            match item {
                Out::Audio(command) => {
                    if self.audio.send(command).is_err() {
                        return false;
                    }
                }
                Out::Desktop(command) => {
                    let _ = self.desktop.send(command);
                }
                Out::Leds(frame) => self.leds.send(&frame),
                Out::SaveState(state) => state.save(&self.state_path),
                Out::HomeAssistant(command) => match &self.home_assistant_calls {
                    Some(calls) => {
                        let _ = calls.send(command);
                    }
                    None => eprintln!("home assistant isn't set up ([home_assistant] url in config.toml)"),
                },
            }
        }
        true
    }
}

/// A knob position from outside the mixer, as the on-screen packet `Core` already understands.
fn set_knob(knob: usize, percent: u8) -> Option<Event> {
    (knob < MAX_CONTROLS).then(|| Event::OnScreen(knob_packet(saturate_u8(knob), percent).to_vec()))
}

/// A button press or release from outside the mixer, as an on-screen packet.
fn press(knob: usize, down: bool) -> Option<Event> {
    (knob < MAX_CONTROLS).then(|| Event::OnScreen(button_packet(saturate_u8(knob), down).to_vec()))
}

/// `--stdin`: each line (`knob 1 40`, `down 0`, `up 0`) is on-screen input.
fn read_stdin(events: &Sender<Event>) {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        match parse_line(&line) {
            Some(bytes) => {
                if events.send(Event::OnScreen(bytes)).is_err() {
                    return;
                }
            }
            None => eprintln!("? {line}"),
        }
    }
}

fn most_knobs(config: &Config) -> usize {
    config.profiles.iter().map(|p| p.channels.len()).max().unwrap_or(0)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().name(name.into()).spawn(work).unwrap();
}

fn sibling(name: &str) -> PathBuf {
    let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&name))).unwrap_or_else(|| name.into())
}

fn spawn_detached(program: &Path, args: &[&std::ffi::OsStr]) {
    use std::process::{Command, Stdio};
    if let Err(e) =
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    {
        eprintln!("can't start {}: {e}", program.display());
    }
}

/// Writes a file only you can read (created that way, never readable by others for a moment).
fn save_secret(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Windows: the settings folder in %APPDATA% is already private to the user
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temporary)?;
    writeln!(file, "{text}")?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::Stream;
    use crate::platform::fake::{self, Recorded};

    /// The fake adapters' recording, kept so a test can look for things in any order.
    struct Recording {
        inbox: Receiver<Recorded>,
        seen: Vec<Recorded>,
    }

    impl Recording {
        /// Whether the service asked for something like `wanted` within a few seconds.
        fn saw(&mut self, wanted: impl Fn(&Recorded) -> bool) -> bool {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if self.seen.iter().any(&wanted) {
                    return true;
                }
                match self.inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(recorded) => self.seen.push(recorded),
                    Err(_) => return false,
                }
            }
        }

        /// Starts looking afresh: what was recorded so far no longer counts.
        fn forget(&mut self) {
            while self.inbox.try_recv().is_ok() {}
            self.seen.clear();
        }
    }

    fn app(index: u32, name: &str, volume: f64) -> Stream {
        Stream { index, app_name: name.into(), volume, channels: 2, writable: true, ..Stream::default() }
    }

    fn sets(index: u32, percent: u8) -> impl Fn(&Recorded) -> bool {
        move |r| matches!(r, Recorded::Audio(audio::Command::SetStream { index: i, percent: p, channels: _ }) if *i == index && *p == percent)
    }

    /// Runs a service on the fake platform with `config`, its files in a temp folder (emptied first, so test runs
    /// don't pile up folders) and no mixer.
    fn start_service(name: &str, config: &str) -> (Sender<Event>, Recording) {
        let dir = std::env::temp_dir().join(format!("cranked-service-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let options = Options {
            config_path: dir.join("config.toml"),
            state_path: dir.join("state.toml"),
            use_mixer: false,
            use_stdin: false,
            verbose: false,
            print_config: false,
            dry_run: false,
            wait_pid: None,
            open_window: false,
        };
        let config = Config::parse(config).unwrap();
        let listener = UnixListener::bind(dir.join("socket")).unwrap();
        let (events, service_inbox) = mpsc::channel();
        let service_events = events.clone();
        let (recording_out, recording_in) = mpsc::channel();
        // the platform isn't Send, so it's made on the service's own thread; runs until the test ends (quitting would
        // end the test program)
        spawn("service", move || {
            let (platform, recorded) = fake::platform();
            let _ = recording_out.send(recorded);
            Service::start(platform, &options, config, listener, service_events).run(&service_inbox);
        });
        let inbox = recording_in.recv().unwrap();
        (events, Recording { inbox, seen: Vec::new() })
    }

    #[test]
    fn a_knob_set_from_outside_turns_its_apps_volume_and_shows_a_popup() {
        let (events, mut recording) = start_service("knob", "[[channel]]\nname = \"Music\"\napps = [\"player\"]\n");
        let player = Stream {
            index: 7,
            app_name: "player".into(),
            volume: 100.0,
            channels: 2,
            writable: true,
            ..Stream::default()
        };
        events.send(Event::AudioReady).unwrap();
        events.send(Event::Stream(player)).unwrap();
        events.send(Event::Remote(dbus::Request::SetKnob { knob: 0, percent: 40 })).unwrap();

        assert!(
            recording.saw(|r| matches!(
                r,
                Recorded::Audio(audio::Command::SetStream { index: 7, percent: 40, channels: _ })
            )),
            "the player is set to 40%: {:?}",
            recording.seen
        );
        assert!(
            recording.saw(|r| matches!(r, Recorded::Desktop(desktop::Command::Volume { percent: 40, name, style: _ }) if name == "Music")),
            "a popup shows Music 40%: {:?}",
            recording.seen
        );
    }

    #[test]
    fn the_tray_follows_profile_switches() {
        let config = "[[profile]]\nname = \"Day\"\n[[profile.channel]]\nname = \"Music\"\n\
                      [[profile]]\nname = \"Night\"\n[[profile.channel]]\nname = \"Quiet\"\n";
        let (events, mut recording) = start_service("tray", config);
        let shows = |profile: &'static str| {
            move |r: &Recorded| {
                matches!(r, Recorded::Tray { profile: shown, profiles, mixer: false }
                    if shown == profile && profiles == &["Day", "Night"])
            }
        };
        // the tray is updated after each event, so the first one shows the starting profile
        events.send(Event::AudioReady).unwrap();
        assert!(recording.saw(shows("Day")), "the tray starts on Day: {:?}", recording.seen);
        events.send(Event::Remote(dbus::Request::NextProfile)).unwrap();
        assert!(recording.saw(shows("Night")), "and follows the switch to Night: {:?}", recording.seen);
        assert!(
            recording.saw(|r| matches!(r, Recorded::Desktop(desktop::Command::Text { text, icon: _, style: _ }) if text == "Profile: Night")),
            "a popup names the new profile: {:?}",
            recording.seen
        );
    }

    #[test]
    fn a_button_tap_reaches_the_media_player_and_a_hold_mutes_its_apps() {
        let config = "[[channel]]\nname = \"Music\"\napps = [\"player\"]\npress = \"play_pause\"\nhold = \"mute\"\n";
        let (events, mut recording) = start_service("buttons", config);
        events.send(Event::AudioReady).unwrap();
        events.send(Event::Stream(app(3, "player", 50.0))).unwrap();
        let button = |down| Event::Remote(dbus::Request::Button { button: 0, down });

        events.send(button(true)).unwrap();
        events.send(button(false)).unwrap();
        let play_pause =
            |r: &Recorded| matches!(r, Recorded::Desktop(desktop::Command::Media(desktop::MediaKey::PlayPause)));
        assert!(recording.saw(play_pause), "a tap is play/pause: {:?}", recording.seen);

        // a person's next press comes later than the bounce guard (presses within 200 ms of the last count as bounce)
        std::thread::sleep(Duration::from_millis(300));
        events.send(button(true)).unwrap();
        std::thread::sleep(crate::buttons::HOLD_AFTER + Duration::from_millis(200));
        events.send(button(false)).unwrap();
        let muted = |r: &Recorded| matches!(r, Recorded::Audio(audio::Command::MuteStream { index: 3, mute: true }));
        assert!(recording.saw(muted), "a hold mutes the player: {:?}", recording.seen);
    }

    #[test]
    fn controller_input_turns_its_knobs_apps() {
        let (events, mut recording) = start_service("controller", "[[channel]]\nname = \"Game\"\napps = [\"game\"]\n");
        events.send(Event::AudioReady).unwrap();
        events.send(Event::Stream(app(4, "game", 100.0))).unwrap();
        events.send(Event::Device(devices::Input::Set { knob: 0, percent: 40 })).unwrap();
        assert!(recording.saw(sets(4, 40)), "a fader sets the game to 40%: {:?}", recording.seen);
        events.send(Event::Device(devices::Input::Step { knob: 0, delta: -5 })).unwrap();
        assert!(recording.saw(sets(4, 35)), "a knob turn steps it to 35%: {:?}", recording.seen);
    }

    #[test]
    fn switching_output_asks_the_sound_system_and_shows_where_it_went() {
        let config = "[[channel]]\nname = \"Output\"\npress = \"switch_output\"\n";
        let (events, mut recording) = start_service("output", config);
        events.send(Event::Remote(dbus::Request::Button { button: 0, down: true })).unwrap();
        events.send(Event::Remote(dbus::Request::Button { button: 0, down: false })).unwrap();
        let asked =
            |r: &Recorded| matches!(r, Recorded::Audio(audio::Command::SwitchOutput { enabled: _, disabled: _ }));
        assert!(recording.saw(asked), "the press asks the sound system to switch: {:?}", recording.seen);

        // what the sound system answers once it switched
        events.send(Event::OutputSwitched(Some("Desk speakers".into()))).unwrap();
        let shown = |r: &Recorded| matches!(r, Recorded::Desktop(desktop::Command::Text { text, icon: _, style: _ }) if text == "Desk speakers");
        assert!(recording.saw(shown), "a popup names the new output: {:?}", recording.seen);
    }

    #[test]
    fn sound_on_a_trigger_app_lowers_the_music_and_brings_it_back() {
        let config = "[auto_lower]\nenabled = true\nlower = [1]\ntriggers = [2]\nvoice_chat = false\namount = 50\n\
                      [[channel]]\nname = \"Music\"\napps = [\"music\"]\n\
                      [[channel]]\nname = \"Talk\"\napps = [\"talk\"]\n";
        let (events, mut recording) = start_service("auto-lower", config);
        events.send(Event::AudioReady).unwrap();
        events.send(Event::Stream(app(1, "music", 60.0))).unwrap();
        events.send(Event::Stream(app(2, "talk", 80.0))).unwrap();
        events.send(Event::Remote(dbus::Request::SetKnob { knob: 0, percent: 60 })).unwrap();
        events.send(Event::Remote(dbus::Request::SetKnob { knob: 1, percent: 80 })).unwrap();
        let metered =
            |r: &Recorded| matches!(r, Recorded::Audio(audio::Command::Meter { index: 2, on: true, sink: _ }));
        assert!(recording.saw(metered), "the trigger app is metered: {:?}", recording.seen);
        assert!(recording.saw(sets(1, 60)), "the music starts at 60%: {:?}", recording.seen);

        // the sound system reports the talk app loud every 200 ms for a while, like someone talking
        for _ in 0..8 {
            events.send(Event::Loud(2)).unwrap();
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(recording.saw(sets(1, 30)), "the music is lowered by half: {:?}", recording.seen);
        recording.forget();
        assert!(recording.saw(sets(1, 60)), "and comes back once it's quiet: {:?}", recording.seen);
    }

    #[test]
    fn an_app_changing_its_own_volume_is_put_back() {
        let (events, mut recording) = start_service("guard", "[[channel]]\nname = \"Chat\"\napps = [\"chat\"]\n");
        events.send(Event::AudioReady).unwrap();
        events.send(Event::Stream(app(5, "chat", 100.0))).unwrap();
        events.send(Event::Remote(dbus::Request::SetKnob { knob: 0, percent: 30 })).unwrap();
        assert!(recording.saw(sets(5, 30)), "the knob sets the chat app to 30%: {:?}", recording.seen);
        recording.forget();
        // the sound system reports the app at 90% (it raised itself)
        events.send(Event::Stream(app(5, "chat", 90.0))).unwrap();
        assert!(recording.saw(sets(5, 30)), "it's put back to the knob's 30%: {:?}", recording.seen);
    }
}
