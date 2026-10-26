//! The service's own view of settings: Home Assistant access and what it remembers between runs
//! (`~/.local/state/cranked-audio/state.toml`, so the settings file is never rewritten behind the user's back).

use std::path::{Path, PathBuf};

pub use cranked_ipc::settings::*;
use serde::{Deserialize, Serialize};

/// The Home Assistant address and token, when it's set up. Warns about a token others can read.
pub fn home_assistant_settings(config: &Config, config_path: &Path) -> Option<crate::home_assistant::Settings> {
    let url = config.home_assistant.url.as_deref()?.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        return None;
    }
    let token_path = config.home_assistant.token_file.clone().unwrap_or_else(|| config_path.with_file_name("ha-token"));
    let token = match std::fs::read_to_string(&token_path) {
        Ok(token) => token.trim().to_string(),
        Err(e) => {
            eprintln!("home assistant: can't read the token at {} ({e})", token_path.display());
            return None;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if std::fs::metadata(&token_path).is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
            eprintln!("home assistant: {} is readable by other users; run chmod 600 on it", token_path.display());
        }
    }
    Some(crate::home_assistant::Settings { url, token })
}

/// What the app remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub profile: String,
    /// Last knob positions by knob number ("1".."5"), used when there's no mixer.
    pub knobs: std::collections::BTreeMap<String, u8>,
    /// App names seen playing, newest last (offered when picking a knob's apps).
    pub apps: Vec<String>,
    /// A mixer has been connected at some point (until then its settings stay out of the way).
    #[serde(default)]
    pub mixer_seen: bool,
}

impl State {
    pub fn default_path() -> PathBuf {
        base_dir("XDG_STATE_HOME", ".local/state").join("state.toml")
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let Ok(text) = toml::to_string(self) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temporary = path.with_extension("toml.new");
        if std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path)).is_err() {
            eprintln!("state: can't write {}", path.display());
        }
    }
}
