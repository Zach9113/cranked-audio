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
mod platform;
mod serial;
mod server;
mod service;
mod shortcuts;
mod tray;
mod updater;

use std::path::PathBuf;
use std::sync::mpsc;

use cranked_protocol::{button_packet, knob_packet};

use crate::config::Config;
use crate::service::Service;

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

/// The command line.
#[expect(clippy::struct_excessive_bools, reason = "independent command-line switches")]
pub struct Options {
    pub config_path: PathBuf,
    /// What the app remembers between runs (`--config`'s counterpart, kept internal).
    pub state_path: PathBuf,
    pub use_mixer: bool,
    pub use_stdin: bool,
    pub verbose: bool,
    pub print_config: bool,
    pub dry_run: bool,
    /// Set by the old service when it restarts into an update (Windows): wait for that process to exit first.
    pub wait_pid: Option<u32>,
    /// Open the window once started (the old service had one open).
    pub open_window: bool,
}

impl Options {
    /// None when an argument isn't understood.
    fn parse(mut args: impl Iterator<Item = String>) -> Option<Self> {
        let mut options = Self {
            config_path: Config::default_path(),
            state_path: config::State::default_path(),
            use_mixer: true,
            use_stdin: false,
            verbose: false,
            print_config: false,
            dry_run: false,
            wait_pid: None,
            open_window: false,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--config" => options.config_path = PathBuf::from(args.next()?),
                "--no-mixer" => options.use_mixer = false,
                "--stdin" => options.use_stdin = true,
                "-v" | "--verbose" => options.verbose = true,
                "--print-config" => options.print_config = true,
                "--dry-run" => options.dry_run = true,
                "--wait-pid" => options.wait_pid = Some(args.next()?.parse().ok()?),
                "--open-window" => options.open_window = true,
                _ => return None,
            }
        }
        Some(options)
    }
}

fn main() {
    let Some(options) = Options::parse(std::env::args().skip(1)) else {
        return eprintln!("{USAGE}");
    };
    let config = match Config::load(&options.config_path) {
        Ok(config) => config,
        Err(e) => return eprintln!("settings: {e}"),
    };
    if options.print_config {
        return print!("{}", toml::to_string(&config).unwrap_or_default());
    }
    // after an update, the old service may still hold the socket and the mixer's port for a moment
    #[cfg(windows)]
    {
        if let Some(pid) = options.wait_pid {
            updater::wait_for_exit(pid, std::time::Duration::from_secs(15));
        }
        updater::remove_leftovers();
    }
    let Some(listener) = server::listen() else {
        return eprintln!("Cranked Audio is already running");
    };
    let (events, inbox) = mpsc::channel();
    Service::start(platform::native(), &options, config, listener, events).run(&inbox);
}

#[cfg(test)]
mod tests {
    use super::{parse_line, Options};

    fn options(args: &[&str]) -> Option<Options> {
        Options::parse(args.iter().map(ToString::to_string))
    }

    #[test]
    fn command_line_is_parsed() {
        let parsed = options(&["--no-mixer", "-v", "--config", "/tmp/x.toml", "--dry-run"]).unwrap();
        assert!(!parsed.use_mixer && parsed.verbose && parsed.dry_run && !parsed.use_stdin && !parsed.print_config);
        assert_eq!(parsed.config_path, std::path::PathBuf::from("/tmp/x.toml"));
        assert!(options(&[]).unwrap().use_mixer);
        assert!(options(&["--config"]).is_none(), "--config needs a file");
        assert!(options(&["--bogus"]).is_none());
        let restarted = options(&["-v", "--wait-pid", "4242", "--open-window"]).unwrap();
        assert_eq!(restarted.wait_pid, Some(4242));
        assert!(restarted.open_window && restarted.verbose);
        assert!(options(&["--wait-pid"]).is_none(), "--wait-pid needs a process id");
        assert!(options(&["--wait-pid", "x"]).is_none());
    }

    #[test]
    fn stdin_lines_become_packets() {
        assert_eq!(parse_line("knob 1 100"), Some(vec![0xFF, 0xFE, 0x03, 1, 0x03, 0xE8]));
        assert_eq!(parse_line("down 4"), Some(vec![0xFF, 0xFE, 0x06, 4]));
        assert_eq!(parse_line("knob 20 10"), None);
        assert_eq!(parse_line("jump 1"), None);
    }
}
