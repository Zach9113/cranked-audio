//! The one place the daemon picks its operating system. Each port is a trait the rest of the daemon talks to;
//! `native()` plugs in this platform's adapters. The plan and the remaining ports: docs/platform-ports.md.

use std::sync::Arc;

use crate::audio::AudioSystem;

/// This platform's adapters, one per port.
pub struct Platform {
    /// Shared: its `run` owns the audio thread while the service lists outputs from the main thread.
    pub audio: Arc<dyn AudioSystem>,
}

/// The adapters for the operating system this daemon was built for.
pub fn native() -> Platform {
    #[cfg(not(windows))]
    let audio = Arc::new(crate::audio::pulse::PipeWire);
    #[cfg(windows)]
    let audio = Arc::new(crate::audio::wasapi::Wasapi);
    Platform { audio }
}
