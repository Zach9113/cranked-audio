//! cranked-audiod: the always-running part of Cranked Audio. No window; the window talks to it.

mod audio;
mod buttons;
mod config;
mod core;
mod desktop;
mod duck;
mod ha;
mod leds;
mod matching;
mod serial;
mod server;
mod tray;

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use cranked_ipc::ServerMessage;
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
    HaState(ha::EntityState),
    HaFailed,
    HaEntities(Vec<cranked_ipc::Entity>),
    /// A watched stream is playing above the Auto-lower threshold.
    Loud(u32),
    /// A window connected / sent a line.
    ClientConnected(std::os::unix::net::UnixStream),
    ClientLine(String),
    Tray(tray::Action),
    AudioReady,
    AudioLost,
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
    let channel: u8 = words.next()?.parse().ok().filter(|c| usize::from(*c) < cranked_protocol::KNOBS)?;
    match verb {
        "knob" => Some(knob_packet(channel, words.next()?.parse().ok()?).to_vec()),
        "down" => Some(button_packet(channel, true).to_vec()),
        "up" => Some(button_packet(channel, false).to_vec()),
        _ => None,
    }
}

fn main() {
    let mut config_path = Config::default_path();
    let (mut use_mixer, mut use_stdin, mut verbose, mut print_config, mut dry_run) = (true, false, false, false, false);
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

    let Some(listener) = server::listen() else {
        return eprintln!("Cranked Audio is already running");
    };
    let state_path = State::default_path();
    let state = State::load(&state_path);

    let (events, inbox) = mpsc::channel();
    let (commands, audio_inbox) = mpsc::channel();
    let (desktop, desktop_inbox) = mpsc::channel();
    let leds = serial::Writer::default();
    let ha_generation = Arc::new(AtomicU64::new(0));
    let start_ha = |config: &Config| -> Option<mpsc::Sender<ha::Command>> {
        let mine = ha_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let settings = config::ha_settings(config, &config_path)?;
        let (calls, calls_inbox) = mpsc::channel();
        let (states, watched, generation) = (events.clone(), config.ha_entities(), Arc::clone(&ha_generation));
        let for_states = settings.clone();
        std::thread::Builder::new()
            .name("ha-states".into())
            .spawn(move || ha::run_states(for_states, watched, states, generation, mine))
            .unwrap();
        let failures = events.clone();
        std::thread::Builder::new().name("ha-calls".into()).spawn(move || ha::run_calls(settings, calls_inbox, failures)).unwrap();
        Some(calls)
    };
    let mut ha_calls = start_ha(&config);
    std::thread::Builder::new().name("desktop".into()).spawn(move || desktop::run(desktop_inbox)).unwrap();
    {
        let events = events.clone();
        std::thread::Builder::new().name("audio".into()).spawn(move || audio::run(audio_inbox, events)).unwrap();
    }
    if use_mixer {
        let events = events.clone();
        let leds = leds.clone();
        std::thread::Builder::new().name("mixer".into()).spawn(move || serial::run(events, leds)).unwrap();
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
        std::thread::Builder::new().name("windows".into()).spawn(move || server::run(listener, events)).unwrap();
    }
    let modified = |path: &PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut config_modified: Option<SystemTime> = modified(&config_path);
    let mut config_checked = Instant::now();
    let mut clients = server::Clients::default();

    let mut core = Core::new(config, &state);
    core.verbose = verbose;
    core.ha_enabled = ha_calls.is_some();
    let tray = tray::start(tray::Tray { events: events.clone(), profile: String::new(), profiles: Vec::new(), mixer: false });
    let mut tray_shows = (String::new(), Vec::<String>::new(), false);
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
        let mut restart_ha = false;
        let event = match event {
            Some(Event::ClientConnected(stream)) => {
                clients.add(stream, &core.snapshot());
                None
            }
            Some(Event::ClientLine(line)) => {
                let (verb, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
                match verb {
                    "reload" => reload = true,
                    "get-config" => clients.send(&ServerMessage::Config(core.config().clone())),
                    "entities" => match &ha_calls {
                        Some(calls) => {
                            let _ = calls.send(ha::Command::ListEntities);
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
                            Ok(()) => restart_ha = true,
                            Err(e) => eprintln!("home assistant: can't save the token ({e})"),
                        }
                    }
                    "profile" => switch_to = Some(rest.to_string()),
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
                        if clients.is_empty() {
                            spawn_detached(&sibling("cranked-audio"), &[]);
                        }
                    }
                    tray::Action::Profile(name) => switch_to = Some(name),
                    tray::Action::EditSettings => {
                        if !config_path.exists() {
                            let _ = core.config().save(&config_path);
                        }
                        spawn_detached(std::path::Path::new("xdg-open"), &[config_path.as_os_str()]);
                    }
                    tray::Action::Quit => {
                        eprintln!("quit from the tray");
                        leds.send(&led_frame(&[[(0, 0, 0); cranked_protocol::LEDS_PER_KNOB]; cranked_protocol::KNOBS]));
                        let _ = std::fs::remove_file(cranked_ipc::socket_path());
                        std::process::exit(0);
                    }
                }
                None
            }
            Some(Event::HaEntities(entities)) => {
                clients.send(&ServerMessage::Entities(entities));
                None
            }
            other => other,
        };
        let mut out = event.map(|event| core.handle(event, now)).unwrap_or_default();
        if let Some(name) = switch_to {
            core.switch_profile(&name, &mut out);
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
            let ha_changed = config.home_assistant != core.config().home_assistant || config.ha_entities() != core.config().ha_entities();
            restart_ha |= ha_changed;
            core.set_config(config, &mut out);
            clients.send(&ServerMessage::Config(core.config().clone()));
        }
        if restart_ha {
            ha_calls = start_ha(core.config());
            core.ha_enabled = ha_calls.is_some();
        }
        out.extend(core.tick(now));
        if !clients.is_empty() {
            clients.update(&core.snapshot());
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
                Out::Ha(command) => match &ha_calls {
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
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).unwrap_or_else(|| name.into())
}

fn spawn_detached(program: &std::path::Path, args: &[&std::ffi::OsStr]) {
    use std::process::{Command, Stdio};
    if let Err(e) = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        eprintln!("can't start {}: {e}", program.display());
    }
}

/// Writes a file only you can read (created that way, never readable by others for a moment).
fn save_secret(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    writeln!(file, "{text}")?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn stdin_lines_become_packets() {
        assert_eq!(parse_line("knob 1 100"), Some(vec![0xFF, 0xFE, 0x03, 1, 0x03, 0xFF]));
        assert_eq!(parse_line("down 4"), Some(vec![0xFF, 0xFE, 0x06, 4]));
        assert_eq!(parse_line("knob 5 10"), None);
        assert_eq!(parse_line("jump 1"), None);
    }
}
