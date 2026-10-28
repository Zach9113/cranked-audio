//! In-app updates: the `Updater` port, and what every platform shares (finding the latest release, downloading,
//! reading checksums). Installing is each platform's own: Linux runs the release's install.sh
//! (`platform::linux::updater`), Windows swaps the two .exe files and restarts (`platform::windows::updater`).

use std::sync::mpsc::Sender;

use serde_json::Value;

use crate::Event;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "Zach9113/cranked-audio";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    /// This platform's package: the Linux tarball or the Windows zip.
    pub package_url: String,
    /// The release's `<package>.sha256`; an update without one is never installed.
    pub checksum_url: Option<String>,
}

/// Compares two semver strings (e.g., "v0.2.0" and "0.1.0").
/// Returns true only if `latest` is strictly newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Option<(u32, u32, u32)> {
        let s = s.trim().trim_start_matches('v');
        let mut parts = s.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next()?.parse().ok()?;
        let patch_str = parts.next().unwrap_or("0").split('-').next()?;
        let patch: u32 = patch_str.parse().ok()?;
        Some((major, minor, patch))
    };

    match (parse(latest), parse(current)) {
        (Some(l), Some(c)) => l > c,
        // a version we can't read is never "newer" (that could install an older or odd build)
        _ => false,
    }
}

/// The port to installing updates. Each platform has one adapter (`linux::updater::InstallScript`,
/// `windows::updater::ExeSwap`), chosen in `platform::native()`.
pub trait Updater {
    /// Looks for a newer release with this platform's package, on its own thread; answers with
    /// `Event::UpdateCheckResult`.
    fn check(&self, events: Sender<Event>);
    /// Whether this copy can install updates itself.
    fn can_install(&self) -> bool;
    /// Downloads, verifies and installs `release` on its own thread; answers with `Event::UpdateApplyResult`.
    fn install(&self, release: ReleaseInfo, events: Sender<Event>);
    /// Whether the service starts the new programs itself once an update is in (else the installer restarts it).
    fn restarts_service(&self) -> bool;
    /// At startup: wait for the service this one replaced (`--wait-pid`) and clear what the update left behind.
    fn after_restart(&self, old_pid: Option<u32>);
}

/// The latest release, if it's newer and carries this platform's package (its name ends with `package_suffix`;
/// `fallback_url` names one for a release that lists none).
pub fn check_for_update(
    package_suffix: &str,
    fallback_url: fn(&str) -> Option<String>,
) -> Result<Option<ReleaseInfo>, String> {
    // CRANKED_UPDATE_API: point the check at a test release (like CRANKED_SOCKET, for testing only)
    let url = std::env::var("CRANKED_UPDATE_API")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let resp = ureq::get(&url)
        .header("User-Agent", "cranked-audio-updater")
        .header("Accept", "application/vnd.github.v3+json")
        .call();

    let mut response = match resp {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => {
            // No releases published yet on GitHub
            return Ok(None);
        }
        Err(e) => return Err(format!("Could not reach update server ({e})")),
    };

    let text = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let tag = data.get("tag_name").and_then(Value::as_str).ok_or("No release tag found")?;

    if !is_newer(tag, CURRENT_VERSION) {
        return Ok(None);
    }

    let assets: Vec<(&str, &str)> = data
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a.get("name")?.as_str()?, a.get("browser_download_url")?.as_str()?)))
        .collect();
    let find = |suffix: &str| assets.iter().find(|(name, _)| name.ends_with(suffix)).map(|(_, url)| url.to_string());
    let Some(package_url) = find(package_suffix).or_else(|| fallback_url(tag)) else {
        // a release without this platform's package: nothing to update to here
        return Ok(None);
    };

    Ok(Some(ReleaseInfo {
        version: tag.to_string(),
        package_url,
        checksum_url: find(&format!("{package_suffix}.sha256")),
    }))
}

pub fn download(url: &str, to: &std::path::Path) -> Result<(), String> {
    let mut response = ureq::get(url)
        .header("User-Agent", "cranked-audio-updater")
        .call()
        .map_err(|e| format!("Download failed ({e})"))?;
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    std::io::copy(&mut response.body_mut().as_reader(), &mut file).map_err(|e| e.to_string())?;
    Ok(())
}

/// The first word of a `sha256sum` line, if it looks like a SHA-256.
pub fn checksum_in(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?.to_ascii_lowercase();
    (word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit())).then_some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.2.0", "v0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
    }

    #[test]
    fn checksums_and_odd_versions() {
        let sum = "a".repeat(64);
        assert_eq!(checksum_in(&format!("{sum}  cranked-audio-v0.2.0-x86_64.tar.gz\n")), Some(sum));
        assert_eq!(checksum_in("not a checksum"), None);
        assert!(!is_newer("nightly", "0.1.0"), "unreadable versions are never newer");
    }
}
