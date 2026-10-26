//! In-app update checking and installation. Installing is Linux-only for now (it runs the release's install.sh);
//! on Windows the app only says that a newer version is out.
#![cfg_attr(windows, allow(dead_code, reason = "the installer part is Linux-only"))]

use serde_json::Value;
use std::process::Command;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "Zach9113/cranked-audio";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub tarball_url: String,
    /// The release's `<tarball>.sha256`; an update without one is never installed.
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

pub fn check_for_update() -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
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
    let tarball_url = find("-x86_64.tar.gz").unwrap_or_else(|| {
        format!("https://github.com/{REPO}/releases/download/{tag}/cranked-audio-{tag}-x86_64.tar.gz")
    });

    Ok(Some(ReleaseInfo { version: tag.to_string(), tarball_url, checksum_url: find("-x86_64.tar.gz.sha256") }))
}

/// Whether this build can install updates itself.
pub const CAN_INSTALL: bool = cfg!(not(windows));

/// A fresh folder only this user can open (never in a shared place like /tmp, never one that already existed),
/// so nothing else on the machine can swap the installer between unpacking and running it.
#[cfg(not(windows))]
fn private_dir() -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .ok_or("no private folder to unpack the update in")?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = base.join(format!("cranked-update-{}-{nanos:x}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("can't create {} ({e})", dir.display()))?;
    Ok(dir)
}

fn download(url: &str, to: &std::path::Path) -> Result<(), String> {
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

fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    let output = Command::new("sha256sum").arg(path).output().map_err(|e| format!("sha256sum: {e}"))?;
    checksum_in(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| "sha256sum gave no checksum".to_string())
}

#[cfg(windows)]
pub fn apply_update(_release: &ReleaseInfo) -> Result<(), String> {
    Err("installing updates from the app isn't available on Windows yet".into())
}

#[cfg(not(windows))]
pub fn apply_update(release: &ReleaseInfo) -> Result<(), String> {
    let checksum_url =
        release.checksum_url.as_deref().ok_or("This release has no checksum, so it won't be installed")?;
    let dir = private_dir()?;
    let result = install_from(&dir, &release.tarball_url, checksum_url);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn install_from(dir: &std::path::Path, tarball_url: &str, checksum_url: &str) -> Result<(), String> {
    let tmp_dir = dir.to_string_lossy().into_owned();
    let tar_path = format!("{tmp_dir}/update.tar.gz");
    let sum_path = dir.join("update.tar.gz.sha256");
    download(checksum_url, &sum_path)?;
    let expected = std::fs::read_to_string(&sum_path)
        .ok()
        .as_deref()
        .and_then(checksum_in)
        .ok_or("The release's checksum file is unreadable")?;

    download(tarball_url, std::path::Path::new(&tar_path))?;
    if sha256_of(std::path::Path::new(&tar_path))? != expected {
        return Err("The download doesn't match the release's checksum; nothing was installed".into());
    }

    // Extract
    let status = Command::new("tar")
        .args(["-xzf", &tar_path, "-C", &tmp_dir])
        .status()
        .map_err(|e| format!("Extraction failed ({e})"))?;

    if !status.success() {
        return Err("Extraction failed".into());
    }

    // Locate install.sh
    let mut installer = None;
    let direct = std::path::PathBuf::from(format!("{tmp_dir}/install.sh"));
    if direct.exists() {
        installer = Some(direct);
    } else if let Ok(entries) = std::fs::read_dir(&tmp_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path().join("install.sh");
            if path.exists() {
                installer = Some(path);
                break;
            }
        }
    }

    let installer = installer.ok_or("Package does not contain install.sh")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&installer) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&installer, perms);
        }
    }

    // Execute install.sh
    let install_status = Command::new(&installer).status().map_err(|e| format!("Installer execution failed ({e})"))?;

    if !install_status.success() {
        return Err("Install script exited with an error".into());
    }

    Ok(())
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
