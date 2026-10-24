//! In-app update checking and installation.

use std::process::Command;
use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "Zach9113/cranked-audio";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub tarball_url: String,
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
        _ => latest.trim_start_matches('v') != current.trim_start_matches('v'),
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

    let mut tarball_url = None;
    if let Some(assets) = data.get("assets").and_then(Value::as_array) {
        for asset in assets {
            if let Some(name) = asset.get("name").and_then(Value::as_str) {
                if name.ends_with("-x86_64.tar.gz") {
                    if let Some(download) = asset.get("browser_download_url").and_then(Value::as_str) {
                        tarball_url = Some(download.to_string());
                        break;
                    }
                }
            }
        }
    }

    let tarball_url = tarball_url.unwrap_or_else(|| {
        format!("https://github.com/{REPO}/releases/download/{tag}/cranked-audio-{tag}-x86_64.tar.gz")
    });

    Ok(Some(ReleaseInfo {
        version: tag.to_string(),
        tarball_url,
    }))
}

pub fn apply_update(tarball_url: &str) -> Result<(), String> {
    let tmp_dir = format!("/tmp/cranked-update-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

    let tar_path = format!("{tmp_dir}/update.tar.gz");

    // Download
    let mut response = ureq::get(tarball_url)
        .header("User-Agent", "cranked-audio-updater")
        .call()
        .map_err(|e| format!("Download failed ({e})"))?;

    let mut file = std::fs::File::create(&tar_path).map_err(|e| e.to_string())?;
    let mut reader = response.body_mut().as_reader();
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;

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
    let install_status = Command::new(&installer)
        .status()
        .map_err(|e| format!("Installer execution failed ({e})"))?;

    let _ = std::fs::remove_dir_all(&tmp_dir);

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
}
