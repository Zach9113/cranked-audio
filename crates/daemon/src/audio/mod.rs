//! The sound system: stream events in, volume changes out. PipeWire (through its PulseAudio layer) on Linux,
//! WASAPI audio sessions on Windows; both report the same `Event`s and take the same `Command`s.

#[cfg(not(windows))]
mod pulse;
#[cfg(not(windows))]
pub use pulse::{list_outputs, run};
#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use wasapi::{list_outputs, run};

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Set a stream's volume in percent (all of its channels).
    SetStream {
        index: u32,
        channels: u8,
        percent: u8,
    },
    /// Set the default output's volume in percent.
    SetMaster {
        percent: u8,
    },
    MuteStream {
        index: u32,
        mute: bool,
    },
    MuteMaster {
        mute: bool,
    },
    /// Make the next enabled output device the default (apps follow); answers with `Event::OutputSwitched`.
    SwitchOutput {
        enabled: Vec<String>,
        disabled: Vec<String>,
    },
    /// Start or stop watching how loud a stream plays (it reports `Event::Loud` while it is).
    Meter {
        index: u32,
        sink: u32,
        on: bool,
    },
}

/// Level readings per second for each watched stream.
const METER_RATE: u32 = 25;
/// About -44 dBFS: quieter than speech or music, louder than silence and line noise.
const LOUD: f32 = 0.0063;
/// Readings in a row above `LOUD` before it counts (two readings = 80 ms).
const LOUD_READINGS: u8 = 2;
/// While loud, report at most this often.
const REPORT_EVERY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    /// A real device (sound card, Bluetooth, USB), not a virtual sink.
    pub real: bool,
}

impl Output {
    pub fn is_enabled(&self, enabled: &[String], disabled: &[String]) -> bool {
        cranked_ipc::settings::output_enabled(&self.description, &self.name, self.real, enabled, disabled)
    }
}

/// The enabled output after `current` (wrapping), or None when there's nothing else to switch to.
pub fn next_output<'a>(
    outputs: &'a [Output],
    current: &str,
    enabled: &[String],
    disabled: &[String],
) -> Option<&'a Output> {
    let chosen: Vec<&Output> = outputs.iter().filter(|o| o.is_enabled(enabled, disabled)).collect();
    let at = chosen.iter().position(|o| o.name == current);
    let next = match at {
        Some(i) => chosen.get((i + 1) % chosen.len().max(1)).copied(),
        None => chosen.first().copied(),
    }?;
    (next.name != current).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, real: bool) -> Output {
        Output { name: name.into(), description: name.to_uppercase(), real }
    }

    #[test]
    fn switching_cycles_enabled_outputs_and_respects_per_output_toggles() {
        let outputs = [output("dac", true), output("nvbroadcast_sink", false), output("hdmi", true)];
        assert_eq!(next_output(&outputs, "dac", &[], &[]).map(|o| o.name.as_str()), Some("hdmi"));
        assert_eq!(next_output(&outputs, "hdmi", &[], &[]).map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs, "nvbroadcast_sink", &[], &[]).map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs[..1], "dac", &[], &[]), None);
        // enabling a virtual output includes it in the cycle; disabling a real output skips it
        let en = ["NVBROADCAST_SINK".to_string()];
        let dis = ["HDMI".to_string()];
        assert_eq!(next_output(&outputs, "dac", &en, &dis).map(|o| o.name.as_str()), Some("nvbroadcast_sink"));
        assert_eq!(next_output(&outputs, "nvbroadcast_sink", &en, &dis).map(|o| o.name.as_str()), Some("dac"));
    }
}
