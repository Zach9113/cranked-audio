//! The session D-Bus API, for the Plasma panel widget, scripts, KDE Connect commands and automations:
//! `io.github.zach9113.CrankedAudio` at `/io/github/zach9113/CrankedAudio`, interface
//! `io.github.zach9113.CrankedAudio1`. State is the same JSON snapshot the window gets.
//!
//! ```text
//! busctl --user call io.github.zach9113.CrankedAudio /io/github/zach9113/CrankedAudio \
//!     io.github.zach9113.CrankedAudio1 SetKnob uu 1 40
//! ```
//!
//! Windows has no session bus; there the API isn't offered (yet).

use std::sync::mpsc::Sender;

use cranked_ipc::Snapshot;

use crate::Event;

/// The port for scripts, the Plasma widget and automations to reach the running app. Each platform has one adapter
/// (`linux::SessionBus`, `windows::NotOffered`), chosen in `platform::native()`.
pub trait RemoteApi {
    /// Offers the API; callers' requests arrive as `Event::Remote`. If it can't be offered it says so once.
    fn start(&mut self, events: Sender<Event>);
    /// Tells listeners the app's state changed.
    fn publish(&self, snapshot: &Snapshot);
}

/// A `remote ...` line from a window or the command line (`cranked-audio --knob 2 +5`) as a request. Knobs and
/// buttons count from 1 here, as people number them.
pub fn remote_request(line: &str) -> Option<RemoteLine> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let number = |word: &str| word.parse::<usize>().ok()?.checked_sub(1);
    let request = match words.as_slice() {
        ["knob", knob, change] => {
            let knob = number(knob)?;
            if change.starts_with(['+', '-']) {
                Request::StepKnob { knob, delta: change.parse().ok()? }
            } else {
                Request::SetKnob { knob, percent: change.parse::<u8>().ok()?.min(100) }
            }
        }
        ["press", button] => return Some(RemoteLine::Press(number(button)?)),
        ["hold", button] => return Some(RemoteLine::Hold(number(button)?)),
        ["next-profile"] => Request::NextProfile,
        ["open-window"] => Request::OpenWindow,
        ["open-settings"] => Request::OpenSettings,
        _ => return None,
    };
    Some(RemoteLine::Request(request))
}

/// What a `remote ...` line asks for: a request, or a button press or hold (which take two requests, down and up).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLine {
    Request(Request),
    Press(usize),
    Hold(usize),
}

/// What a D-Bus caller asked for; the main loop carries it out with the same rules as everything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    SetKnob { knob: usize, percent: u8 },
    StepKnob { knob: usize, delta: i32 },
    Button { button: usize, down: bool },
    Profile(String),
    OpenWindow,
    OpenSettings,
    NextProfile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_lines_count_knobs_from_one() {
        let request = |line| remote_request(line);
        assert_eq!(request("knob 2 +5"), Some(RemoteLine::Request(Request::StepKnob { knob: 1, delta: 5 })));
        assert_eq!(request("knob 1 -3"), Some(RemoteLine::Request(Request::StepKnob { knob: 0, delta: -3 })));
        assert_eq!(request("knob 3 40"), Some(RemoteLine::Request(Request::SetKnob { knob: 2, percent: 40 })));
        let clamped = Some(RemoteLine::Request(Request::SetKnob { knob: 0, percent: 100 }));
        assert_eq!(request("knob 1 250"), clamped, "a level stops at 100%, like SetKnob over D-Bus");
        assert_eq!(request("press 1"), Some(RemoteLine::Press(0)));
        assert_eq!(request("hold 4"), Some(RemoteLine::Hold(3)));
        assert_eq!(request("next-profile"), Some(RemoteLine::Request(Request::NextProfile)));
        assert_eq!(request("knob 0 +5"), None, "there is no knob 0");
        assert_eq!(request("dance"), None);
    }
}
