//! The sound system: stream events in, volume changes out. PipeWire (through its PulseAudio layer) on Linux,
//! WASAPI audio sessions on Windows; both report the same `Event`s and take the same `Command`s.

#[cfg(not(windows))]
mod pulse;
#[cfg(not(windows))]
pub use pulse::run;
#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use wasapi::run;

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
    /// Make the next real output device the default (apps follow); answers with `Event::OutputSwitched`.
    SwitchOutput,
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

/// The real output after `current` (wrapping), or None when there's nothing else to switch to.
pub fn next_output<'a>(outputs: &'a [Output], current: &str) -> Option<&'a Output> {
    let real: Vec<&Output> = outputs.iter().filter(|o| o.real).collect();
    let at = real.iter().position(|o| o.name == current);
    let next = match at {
        Some(i) => real.get((i + 1) % real.len().max(1)).copied(),
        None => real.first().copied(),
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
    fn switching_cycles_real_outputs_only() {
        let outputs = [output("dac", true), output("nvbroadcast_sink", false), output("hdmi", true)];
        assert_eq!(next_output(&outputs, "dac").map(|o| o.name.as_str()), Some("hdmi"));
        assert_eq!(next_output(&outputs, "hdmi").map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs, "nvbroadcast_sink").map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs[..1], "dac"), None);
    }
}
