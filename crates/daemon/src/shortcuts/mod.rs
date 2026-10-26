//! Global shortcuts for the knobs, buttons, next profile and the window: KDE's on Linux, none on Windows (yet).

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::Shortcuts;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Shortcuts;
