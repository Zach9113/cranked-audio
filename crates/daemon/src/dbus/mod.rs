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

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::{publish, start, Published};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{publish, start, Published};

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
