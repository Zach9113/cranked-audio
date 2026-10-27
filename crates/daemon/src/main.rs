//! cranked-audiod: the always-running part of Cranked Audio. No window; the window talks to it.
// Windows: no console window (its log still goes wherever stderr is redirected)
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod audio;
mod buttons;
mod config;
mod core;
mod dbus;
mod desktop;
mod devices;
mod duck;
mod home_assistant;
mod leds;
mod matching;
mod serial;
mod server;
mod shortcuts;
mod tray;
mod updater;

use std::io::BufRead as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use cranked_ipc::ServerMessage;
use cranked_protocol::convert::saturate_u8;
use cranked_protocol::{button_packet, knob_packet, led_frame};

use crate::config::{Config, State};
use crate::core::{Core, Out};

pub enum Event {
    MixerConnected,
    MixerLost,
    /// Raw bytes from the mixer.
    Bytes(Vec<u8>),
    /// Mixer-format packets from the on-screen controls.
    OnScreen(Vec<u8>),
    Stream(matching::Stream),
    StreamRemoved(u32),
    /// The default output changed to this device (None: there was no other one).
    OutputSwitched(Option<String>),
    HomeAssistantState(home_assistant::EntityState),
    HomeAssistantFailed,
    /// The screen is locked or every monitor sleeps (true), or it's back (false).
    ScreenOff(bool),
    HomeAssistantEntities(Vec<cranked_ipc::Entity>),
    /// A watched stream is playing above the Auto-lower threshold.
    Loud(u32),
    /// A window connected, disconnected or sent a line.
    ClientConnected(u64, cranked_ipc::UnixStream),
    ClientDisconnected(u64),
    ClientLine(String),
    Tray(tray::Action),
    /// A request over the D-Bus API.
    Remote(dbus::Request),
    /// Input from a keyboard knob, MIDI controller or deej box.
    Device(devices::Input),
    /// What the MIDI learn mode heard (None: nothing moved in time).
    MidiLearned(Option<devices::Learned>),
    AudioReady,
    AudioLost,
    /// The default output's volume when audio connected.
    OutputVolume(u8),
    UpdateCheckResult(Result<Option<updater::ReleaseInfo>, String>),
    UpdateApplyResult(Result<(), String>),
}

const USAGE: &str = "usage: cranked-audiod [--config FILE] [--no-mixer] [--stdin] [-v]
  --config FILE  settings file (default ~/.config/cranked-audio/config.toml)
  --no-mixer     don't open the mixer's serial port
  --stdin        read on-screen input from stdin: `knob CH PERCENT`, `down CH`, `up CH`
  --dry-run      decide everything but change no volumes (use with -v)
  --print-config show the settings as loaded and exit
  -v             log what happens";

/// `knob 1 40` / `down 0` / `up 0` as mixer packets.
fn parse_line(line: &str) -> Option<Vec<u8>> {
    let mut words = line.split_whitespace();
    let verb = words.next()?;
    let channel: u8 = words.next()?.parse().ok().filter(|c| usize::from(*c) < cranked_ipc::settings::MAX_CONTROLS)?;
    match verb {
        "knob" => Some(knob_packet(channel, words.next()?.parse().ok()?).to_vec()),
        "down" => Some(button_packet(channel, true).to_vec()),
        "up" => Some(button_packet(channel, false).to_vec()),
        _ => None,
    }
}

#[expect(clippy::cognitive_complexity, reason = "startup plus the event loop; splitting it up is its own change")]
fn main() {
    let mut config_path = Config::default_path();
    let (mut use_mixer, mut use_stdin, mut verbose, mut print_config, mut dry_run) = (true, false, false, false, false);
    // set by the old service when it restarts into an update
    let (mut wait_pid, mut open_window): (Option<u32>, bool) = (None, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => match args.next() {
                Some(path) => config_path = PathBuf::from(path),
                None => return eprintln!("{USAGE}"),
            },
            "--no-mixer" => use_mixer = false,
            "--stdin" => use_stdin = true,
            "-v" | "--verbose" => verbose = true,
            "--print-config" => print_config = true,
            "--dry-run" => dry_run = true,
            "--wait-pid" => match args.next().and_then(|pid| pid.parse().ok()) {
                Some(pid) => wait_pid = Some(pid),
                None => return eprintln!("{USAGE}"),
            },
            "--open-window" => open_window = true,
            _ => return eprintln!("{USAGE}"),
        }
    }
    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(e) => return eprintln!("settings: {e}"),
    };
    if print_config {
        return print!("{}", toml::to_string(&config).unwrap_or_default());
    }

    #[cfg(windows)]
    {
        if let Some(pid) = wait_pid {
            updater::wait_for_exit(pid, Duration::from_secs(15));
        }
        updater::remove_leftovers();
    }
    #[cfg(not(windows))]
    let _ = wait_pid;
    let Some(listener) = server::listen() else {
        return eprintln!("Cranked Audio is already running");
    };
    if open_window {
        spawn_detached(&sibling("cranked-audio"), &[]);
    }
    let state_path = State::default_path();
    let state = State::load(&state_path);

    let (events, inbox) = mpsc::channel();
    let (commands, audio_inbox) = mpsc::channel();
    let (desktop, desktop_inbox) = mpsc::channel();
    let leds = serial::Writer::default();
    let home_assistant_generation = Arc::new(AtomicU64::new(0));
    let start_home_assistant = |config: &Config| -> Option<mpsc::Sender<home_assistant::Command>> {
        let mine = home_assistant_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let settings = config::home_assistant_settings(config, &config_path)?;
        let (calls, calls_inbox) = mpsc::channel();
        let (states, watched, generation) =
            (events.clone(), config.home_assistant_entities(), Arc::clone(&home_assistant_generation));
        let for_states = settings.clone();
        std::thread::Builder::new()
            .name("home-assistant-states".into())
            .spawn(move || home_assistant::run_states(&for_states, &watched, &states, &generation, mine))
            .unwrap();
        let failures = events.clone();
        std::thread::Builder::new()
            .name("home-assistant-calls".into())
            .spawn(move || home_assistant::run_calls(&settings, &calls_inbox, &failures))
            .unwrap();
        Some(calls)
    };
    let mut home_assistant_calls = start_home_assistant(&config);
    std::thread::Builder::new().name("desktop".into()).spawn(move || desktop::run(desktop_inbox)).unwrap();
    {
        let events = events.clone();
        std::thread::Builder::new().name("screen".into()).spawn(move || desktop::watch_screen(&events)).unwrap();
    }
    {
        let events = events.clone();
        std::thread::Builder::new().name("audio".into()).spawn(move || audio::run(&audio_inbox, &events)).unwrap();
    }
    if use_mixer {
        let events = events.clone();
        let leds = leds.clone();
        std::thread::Builder::new().name("mixer".into()).spawn(move || serial::run(&events, &leds)).unwrap();
    }
    if use_stdin {
        let events = events.clone();
        std::thread::Builder::new()
            .name("stdin".into())
            .spawn(move || {
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
            })
            .unwrap();
    }
    {
        let events = events.clone();
        std::thread::Builder::new().name("windows".into()).spawn(move || server::run(&listener, &events)).unwrap();
    }
    let modified = |path: &PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut config_modified: Option<SystemTime> = modified(&config_path);
    let mut config_checked = Instant::now();
    let mut clients = server::Clients::default();

    let mut core = Core::new(config, &state);
    core.verbose = verbose;
    core.home_assistant_enabled = home_assistant_calls.is_some();
    let api = dbus::start(events.clone());
    let device_generation = Arc::new(AtomicU64::new(0));
    devices::start(&core.config().devices, &events, &device_generation);
    let most_knobs = |config: &Config| config.profiles.iter().map(|p| p.channels.len()).max().unwrap_or(0);
    let mut shortcuts =
        shortcuts::Shortcuts::start(events.clone(), most_knobs(core.config()), &core.config().shortcuts);
    let tray =
        tray::start(tray::Tray { events: events.clone(), profile: String::new(), profiles: Vec::new(), mixer: false });
    let mut tray_shows = (String::new(), Vec::<String>::new(), false);
    let mut latest_release: Option<updater::ReleaseInfo> = None;
    loop {
        let event = match inbox.recv_timeout(core.next_wake()) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let now = Instant::now();
        let mut reload = false;
        let mut new_config: Option<Config> = None;
        let mut switch_to: Option<String> = None;
        let mut cycle_profile = false;
        let mut restart_home_assistant = false;
        let event = match event {
            Some(Event::ClientConnected(id, stream)) => {
                clients.add(id, stream, &core.snapshot());
                None
            }
            Some(Event::ClientDisconnected(id)) => {
                clients.remove(id);
                None
            }
            Some(Event::ClientLine(line)) => {
                let (verb, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
                match verb {
                    "reload" => reload = true,
                    "get-config" => clients.send(&ServerMessage::Config(core.config().clone())),
                    "entities" => match &home_assistant_calls {
                        Some(calls) => {
                            let _ = calls.send(home_assistant::Command::ListEntities);
                        }
                        None => clients.send(&ServerMessage::Entities(Vec::new())),
                    },
                    "config" => match serde_json::from_str::<Config>(rest) {
                        Ok(mut config) => {
                            config.normalize();
                            match config.save(&config_path) {
                                Ok(()) => {
                                    config_modified = modified(&config_path);
                                    new_config = Some(config);
                                }
                                Err(e) => eprintln!("settings: can't save {} ({e})", config_path.display()),
                            }
                        }
                        Err(e) => eprintln!("settings from the window: {e}"),
                    },
                    "token" => {
                        let path = core.config().token_path(&config_path);
                        match save_secret(&path, rest.trim()) {
                            Ok(()) => restart_home_assistant = true,
                            Err(e) => eprintln!("home assistant: can't save the token ({e})"),
                        }
                    }
                    "profile" => switch_to = Some(rest.to_string()),
                    "list-devices" => {
                        let (keyboards, typing) = devices::keyboard_knobs();
                        clients.send(&ServerMessage::Devices(cranked_ipc::DeviceChoices {
                            keyboards,
                            typing,
                            midi: devices::midi_controllers(),
                            serial: devices::serial_ports(),
                            outputs: audio::list_outputs(),
                        }));
                    }
                    "learn-midi" => {
                        let events = events.clone();
                        std::thread::spawn(move || devices::learn_midi(&events, Duration::from_secs(15)));
                    }
                    // `calibrate R G B` shows the LED calibration pattern with that correction; `calibrate off` ends it
                    "calibrate" => {
                        let values: Vec<f64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                        let gamma = <[f64; 3]>::try_from(values).ok();
                        core.calibrate(gamma, Instant::now());
                    }
                    "check-update" => {
                        let tx = events.clone();
                        std::thread::Builder::new()
                            .name("updater-check".into())
                            .spawn(move || {
                                let res = updater::check_for_update();
                                let _ = tx.send(Event::UpdateCheckResult(res));
                            })
                            .unwrap();
                    }
                    "apply-update" => {
                        if let Some(info) = latest_release.clone() {
                            let tx = events.clone();
                            std::thread::Builder::new()
                                .name("updater-apply".into())
                                .spawn(move || {
                                    let res = updater::apply_update(&info);
                                    let _ = tx.send(Event::UpdateApplyResult(res));
                                })
                                .unwrap();
                        } else {
                            clients.send(&ServerMessage::UpdateStatus {
                                status: "No update found to install. Check for updates first.".into(),
                                latest_version: None,
                                can_install: false,
                            });
                        }
                    }
                    _ => match parse_line(&line) {
                        Some(bytes) => {
                            let _ = events.send(Event::OnScreen(bytes));
                        }
                        None => eprintln!("window: ? {line}"),
                    },
                }
                None
            }
            Some(Event::Tray(action)) => {
                match action {
                    tray::Action::OpenWindow => {
                        if !clients.is_empty() {
                            clients.send(&ServerMessage::OpenPage(String::new()));
                        }
                        if clients.is_empty() {
                            spawn_detached(&sibling("cranked-audio"), &[]);
                        }
                    }
                    tray::Action::Profile(name) => switch_to = Some(name),
                    tray::Action::EditSettings => {
                        if !config_path.exists() {
                            let _ = core.config().save(&config_path);
                        }
                        let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
                        spawn_detached(std::path::Path::new(opener), &[config_path.as_os_str()]);
                    }
                    tray::Action::Quit => {
                        eprintln!("quit from the tray");
                        clients.send(&ServerMessage::Quit);
                        leds.send(&led_frame(&[[(0, 0, 0); cranked_protocol::LEDS_PER_KNOB]; cranked_protocol::KNOBS]));
                        leds.flush();
                        let _ = std::fs::remove_file(cranked_ipc::socket_path());
                        std::process::exit(0);
                    }
                }
                None
            }
            Some(Event::Remote(request)) => match request {
                dbus::Request::SetKnob { knob, percent } if knob < cranked_ipc::settings::MAX_CONTROLS => {
                    Some(Event::OnScreen(knob_packet(saturate_u8(knob), percent).to_vec()))
                }
                dbus::Request::StepKnob { knob, delta } if knob < cranked_ipc::settings::MAX_CONTROLS => {
                    let current = i32::from(core.knob(knob).unwrap_or(0));
                    Some(Event::OnScreen(
                        knob_packet(saturate_u8(knob), saturate_u8((current + delta).clamp(0, 100))).to_vec(),
                    ))
                }
                dbus::Request::Button { button, down } if button < cranked_ipc::settings::MAX_CONTROLS => {
                    Some(Event::OnScreen(button_packet(saturate_u8(button), down).to_vec()))
                }
                dbus::Request::Profile(name) => {
                    switch_to = Some(name);
                    None
                }
                dbus::Request::OpenWindow => {
                    if !clients.is_empty() {
                        clients.send(&ServerMessage::OpenPage(String::new()));
                    }
                    if clients.is_empty() {
                        spawn_detached(&sibling("cranked-audio"), &[]);
                    }
                    None
                }
                dbus::Request::NextProfile => {
                    cycle_profile = true;
                    None
                }
                dbus::Request::OpenSettings => {
                    if clients.is_empty() {
                        spawn_detached(&sibling("cranked-audio"), &["--page".as_ref(), "settings".as_ref()]);
                    } else {
                        clients.send(&ServerMessage::OpenPage("settings".into()));
                    }
                    None
                }
                _ => None,
            },
            Some(Event::Device(input)) => match input {
                devices::Input::Set { knob, percent } if knob < cranked_ipc::settings::MAX_CONTROLS => {
                    Some(Event::OnScreen(knob_packet(saturate_u8(knob), percent).to_vec()))
                }
                devices::Input::Step { knob, delta } if knob < cranked_ipc::settings::MAX_CONTROLS => {
                    let current = i32::from(core.knob(knob).unwrap_or(0));
                    Some(Event::OnScreen(
                        knob_packet(saturate_u8(knob), saturate_u8((current + delta).clamp(0, 100))).to_vec(),
                    ))
                }
                devices::Input::Button { knob, down } if knob < cranked_ipc::settings::MAX_CONTROLS => {
                    Some(Event::OnScreen(button_packet(saturate_u8(knob), down).to_vec()))
                }
                devices::Input::Media(key) => {
                    let _ = desktop.send(desktop::Command::Media(key));
                    None
                }
                _ => None,
            },
            Some(Event::MidiLearned(learned)) => {
                clients.send(&ServerMessage::MidiLearned(learned.map(|l| cranked_ipc::LearnedControl {
                    port: l.port,
                    channel: l.channel,
                    kind: l.kind,
                    number: l.number,
                })));
                None
            }
            Some(Event::HomeAssistantEntities(entities)) => {
                clients.send(&ServerMessage::Entities(entities));
                None
            }
            Some(Event::UpdateCheckResult(res)) => {
                match res {
                    Ok(Some(info)) => {
                        let version = info.version.clone();
                        latest_release = Some(info);
                        clients.send(&ServerMessage::UpdateStatus {
                            status: format!("Update available: {version}"),
                            latest_version: Some(version),
                            can_install: updater::CAN_INSTALL,
                        });
                    }
                    Ok(None) => {
                        latest_release = None;
                        clients.send(&ServerMessage::UpdateStatus {
                            status: format!("Up to date ({})", updater::CURRENT_VERSION),
                            latest_version: None,
                            can_install: false,
                        });
                    }
                    Err(e) => {
                        clients.send(&ServerMessage::UpdateStatus {
                            status: format!("Check failed: {e}"),
                            latest_version: None,
                            can_install: false,
                        });
                    }
                }
                None
            }
            Some(Event::UpdateApplyResult(res)) => {
                match res {
                    Ok(()) => {
                        clients.send(&ServerMessage::UpdateStatus {
                            status: "Update installed! Restarting...".into(),
                            latest_version: None,
                            can_install: false,
                        });
                        // Linux: install.sh restarts the service. Windows: start the new one, which waits for this
                        // one to exit (socket, mixer port), and reopen the window if one was open.
                        #[cfg(windows)]
                        {
                            // the same options this one was started with (--config, -v, ...), minus the restart ones
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
                            let mut args: Vec<&std::ffi::OsStr> = kept.iter().map(|a| a.as_os_str()).collect();
                            args.extend([std::ffi::OsStr::new("--wait-pid"), std::ffi::OsStr::new(&pid)]);
                            if !clients.is_empty() {
                                args.push("--open-window".as_ref());
                            }
                            spawn_detached(&sibling("cranked-audiod"), &args);
                            clients.send(&ServerMessage::Quit);
                            let _ = std::fs::remove_file(cranked_ipc::socket_path());
                            std::process::exit(0);
                        }
                    }
                    Err(e) => {
                        clients.send(&ServerMessage::UpdateStatus {
                            status: format!("Install failed: {e}"),
                            latest_version: None,
                            can_install: false,
                        });
                    }
                }
                None
            }
            other => other,
        };
        let mut out = event.map(|event| core.handle(event, now)).unwrap_or_default();
        if let Some(name) = switch_to {
            core.switch_profile(&name, &mut out);
        }
        if cycle_profile {
            core.cycle_profile(&mut out);
        }
        if now.duration_since(config_checked) >= Duration::from_secs(1) {
            config_checked = now;
            let latest = modified(&config_path);
            reload |= latest != config_modified;
            config_modified = latest;
        }
        if reload {
            match Config::load(&config_path) {
                Ok(config) => new_config = Some(config),
                Err(e) => eprintln!("settings: {e} (keeping the previous settings)"),
            }
        }
        if let Some(config) = new_config {
            eprintln!("settings: applied");
            let home_assistant_changed = config.home_assistant != core.config().home_assistant
                || config.home_assistant_entities() != core.config().home_assistant_entities();
            restart_home_assistant |= home_assistant_changed;
            if let Some(shortcuts) = &mut shortcuts {
                shortcuts.ensure(most_knobs(&config), &config.shortcuts);
            }
            if config.devices != core.config().devices {
                devices::start(&config.devices, &events, &device_generation);
            }
            core.set_config(config, &mut out);
            clients.send(&ServerMessage::Config(core.config().clone()));
        }
        if restart_home_assistant {
            home_assistant_calls = start_home_assistant(core.config());
            core.home_assistant_enabled = home_assistant_calls.is_some();
        }
        out.extend(core.tick(now));
        if !clients.is_empty() || api.is_some() {
            let snapshot = core.snapshot();
            if let Some((connection, state)) = &api {
                dbus::publish(connection, state, &serde_json::to_string(&snapshot).unwrap_or_default());
            }
            if !clients.is_empty() {
                clients.update(&snapshot);
            }
        }
        if let Some(tray) = &tray {
            let snapshot = core.snapshot();
            let shows = (snapshot.profile, snapshot.profiles, snapshot.mixer);
            if shows != tray_shows {
                tray_shows = shows.clone();
                tray.update(move |t| {
                    (t.profile, t.profiles, t.mixer) = shows;
                });
            }
        }
        for item in out {
            if dry_run {
                continue;
            }
            match item {
                Out::Audio(command) => {
                    if commands.send(command).is_err() {
                        return;
                    }
                }
                Out::Desktop(command) => {
                    let _ = desktop.send(command);
                }
                Out::Leds(frame) => leds.send(&frame),
                Out::SaveState(state) => state.save(&state_path),
                Out::HomeAssistant(command) => match &home_assistant_calls {
                    Some(calls) => {
                        let _ = calls.send(command);
                    }
                    None => eprintln!("home assistant isn't set up ([home_assistant] url in config.toml)"),
                },
            }
        }
    }
}

fn sibling(name: &str) -> PathBuf {
    let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&name))).unwrap_or_else(|| name.into())
}

fn spawn_detached(program: &std::path::Path, args: &[&std::ffi::OsStr]) {
    use std::process::{Command, Stdio};
    if let Err(e) =
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    {
        eprintln!("can't start {}: {e}", program.display());
    }
}

/// Writes a file only you can read (created that way, never readable by others for a moment).
fn save_secret(path: &std::path::Path, text: &str) -> std::io::Result<()> {
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
    use super::parse_line;

    #[test]
    fn stdin_lines_become_packets() {
        assert_eq!(parse_line("knob 1 100"), Some(vec![0xFF, 0xFE, 0x03, 1, 0x03, 0xE8]));
        assert_eq!(parse_line("down 4"), Some(vec![0xFF, 0xFE, 0x06, 4]));
        assert_eq!(parse_line("knob 20 10"), None);
        assert_eq!(parse_line("jump 1"), None);
    }
}
