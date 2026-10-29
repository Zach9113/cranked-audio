//! The one place the daemon picks its operating system. Each port is a trait the rest of the daemon talks to (in
//! audio, desktop, tray, shortcuts, dbus and devices); every adapter lives under `linux/` or `windows/` here, and
//! `native()` plugs in this platform's set. The plan: docs/platform-ports.md.

#[cfg(test)]
pub mod fake;
#[cfg(not(windows))]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use crate::audio::AudioSystem;
use crate::dbus::RemoteApi;
use crate::desktop::Desktop;
use crate::devices::InputHardware;
use crate::shortcuts::GlobalShortcuts;
use crate::tray::TrayIcon;
use crate::updater::Updater;

/// This platform's adapters, one per port.
pub struct Platform {
    pub audio: Box<dyn AudioSystem>,
    pub desktop: Box<dyn Desktop>,
    pub tray: Box<dyn TrayIcon>,
    pub shortcuts: Box<dyn GlobalShortcuts>,
    pub remote_api: Box<dyn RemoteApi>,
    pub input_hardware: Box<dyn InputHardware>,
    pub updater: Box<dyn Updater>,
}

/// The adapters for the operating system this daemon was built for.
pub fn native() -> Platform {
    #[cfg(not(windows))]
    let platform = Platform {
        audio: Box::new(linux::audio::PipeWire),
        desktop: Box::new(linux::desktop::Plasma),
        tray: Box::new(linux::tray::StatusNotifier::default()),
        shortcuts: linux::shortcuts::native(),
        remote_api: Box::new(linux::remote_api::SessionBus::default()),
        input_hardware: Box::new(linux::input_hardware::Evdev),
        updater: Box::new(linux::updater::InstallScript),
    };
    #[cfg(windows)]
    let platform = Platform {
        audio: Box::new(windows::audio::Wasapi),
        desktop: Box::new(windows::desktop::Win32),
        tray: Box::new(windows::tray::NotificationArea::default()),
        shortcuts: Box::new(windows::shortcuts::HotKeys::default()),
        remote_api: Box::new(windows::remote_api::NotOffered),
        input_hardware: Box::new(windows::input_hardware::RawInput),
        updater: Box::new(windows::updater::ExeSwap),
    };
    platform
}

/// Starts a named worker thread for an adapter (the name shows up in `top -H` and crash reports).
pub fn spawn_thread(name: &str, work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().name(name.into()).spawn(work).unwrap();
}
