//! The one place the daemon picks its operating system. Each port is a trait the rest of the daemon talks to;
//! `native()` plugs in this platform's adapters. The plan and the remaining ports: docs/platform-ports.md.

use crate::audio::AudioSystem;
use crate::dbus::RemoteApi;
use crate::desktop::Desktop;
use crate::shortcuts::GlobalShortcuts;
use crate::tray::TrayIcon;

/// This platform's adapters, one per port.
pub struct Platform {
    pub audio: Box<dyn AudioSystem>,
    pub desktop: Box<dyn Desktop>,
    pub tray: Box<dyn TrayIcon>,
    pub shortcuts: Box<dyn GlobalShortcuts>,
    pub remote_api: Box<dyn RemoteApi>,
}

/// The adapters for the operating system this daemon was built for.
pub fn native() -> Platform {
    #[cfg(not(windows))]
    let platform = Platform {
        audio: Box::new(crate::audio::pulse::PipeWire),
        desktop: Box::new(crate::desktop::linux::Plasma),
        tray: Box::new(crate::tray::linux::StatusNotifier::default()),
        shortcuts: Box::new(crate::shortcuts::linux::KGlobalAccel::default()),
        remote_api: Box::new(crate::dbus::linux::SessionBus::default()),
    };
    #[cfg(windows)]
    let platform = Platform {
        audio: Box::new(crate::audio::wasapi::Wasapi),
        desktop: Box::new(crate::desktop::windows::Win32),
        tray: Box::new(crate::tray::windows::NotificationArea::default()),
        shortcuts: Box::new(crate::shortcuts::windows::HotKeys::default()),
        remote_api: Box::new(crate::dbus::windows::NotOffered),
    };
    platform
}

/// Starts a named worker thread for an adapter (the name shows up in `top -H` and crash reports).
pub fn spawn_thread(name: &str, work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().name(name.into()).spawn(work).unwrap();
}
