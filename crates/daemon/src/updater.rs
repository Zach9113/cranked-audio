//! In-app update checking and installation. Linux runs the release's install.sh; Windows swaps the two .exe files
//! itself (a running .exe can be renamed, not overwritten) and the service restarts into the new one.

use serde_json::Value;
use std::process::Command;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "Zach9113/cranked-audio";

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

pub fn check_for_update() -> Result<Option<ReleaseInfo>, String> {
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
    #[cfg(not(windows))]
    let package_url = find(PACKAGE_SUFFIX).unwrap_or_else(|| {
        format!("https://github.com/{REPO}/releases/download/{tag}/cranked-audio-{tag}-x86_64.tar.gz")
    });
    #[cfg(windows)]
    let Some(package_url) = find(PACKAGE_SUFFIX) else {
        // a release without a Windows build: nothing to update to here
        return Ok(None);
    };

    Ok(Some(ReleaseInfo {
        version: tag.to_string(),
        package_url,
        checksum_url: find(&format!("{PACKAGE_SUFFIX}.sha256")),
    }))
}

/// The end of this platform's package name among the release's assets.
#[cfg(not(windows))]
const PACKAGE_SUFFIX: &str = "-x86_64.tar.gz";
#[cfg(windows)]
const PACKAGE_SUFFIX: &str = "-windows-x86_64-experimental.zip";

/// Whether this copy can install updates itself. A copy a package manager put in /usr can't: the release's
/// installer would add a second copy in ~/.local/bin, so the package manager updates it instead.
pub fn can_install() -> bool {
    std::env::current_exe().is_ok_and(|program| !is_system_install(&program))
}

/// Whether a program lives in the system's own folders (installed by a package manager, e.g. from the AUR).
pub fn is_system_install(program: &std::path::Path) -> bool {
    cfg!(not(windows)) && program.starts_with("/usr")
}

/// A fresh folder only this user can open (never in a shared place like /tmp, never one that already existed),
/// so nothing else on the machine can swap the installer between unpacking and running it.
#[cfg(not(windows))]
fn private_dir() -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt as _;
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

#[cfg(not(windows))]
fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    let output = Command::new("sha256sum").arg(path).output().map_err(|e| format!("sha256sum: {e}"))?;
    checksum_in(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| "sha256sum gave no checksum".to_string())
}

#[cfg(windows)]
fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest as _, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// The two programs a Windows package carries; both are swapped together.
#[cfg(windows)]
const PROGRAMS: [&str; 2] = ["cranked-audiod.exe", "cranked-audio.exe"];

#[cfg(windows)]
pub fn apply_update(release: &ReleaseInfo) -> Result<(), String> {
    let checksum_url =
        release.checksum_url.as_deref().ok_or("This release has no checksum, so it won't be installed")?;
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        .ok_or("can't tell where Cranked Audio is installed")?;
    // %LOCALAPPDATA%\cranked-audio is the user's own; a new folder each time, never one that already existed
    let base = cranked_ipc::settings::base_dir("XDG_STATE_HOME", ".local/state");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = base.join(format!("update-{}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    #[expect(clippy::create_dir, reason = "a folder that already existed must fail: someone else could have made it")]
    std::fs::create_dir(&dir).map_err(|e| format!("can't create {} ({e})", dir.display()))?;
    let result = install_windows(&dir, &install_dir, &release.package_url, checksum_url);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(windows)]
fn install_windows(
    dir: &std::path::Path,
    install_dir: &std::path::Path,
    package_url: &str,
    checksum_url: &str,
) -> Result<(), String> {
    let sum_path = dir.join("update.zip.sha256");
    download(checksum_url, &sum_path)?;
    let expected = std::fs::read_to_string(&sum_path)
        .ok()
        .as_deref()
        .and_then(checksum_in)
        .ok_or("The release's checksum file is unreadable")?;
    let zip = dir.join("update.zip");
    download(package_url, &zip)?;
    if sha256_of(&zip)? != expected {
        return Err("The download doesn't match the release's checksum; nothing was installed".into());
    }
    // Windows' own tar (bsdtar) reads zips; named by its full path so nothing on PATH stands in for it
    let system = std::env::var_os("SystemRoot").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from);
    let unpacked = dir.join("unpacked");
    #[expect(clippy::create_dir, reason = "inside the fresh folder above; its parent always exists")]
    std::fs::create_dir(&unpacked).map_err(|e| e.to_string())?;
    let status = Command::new(system.join("System32").join("tar.exe"))
        .arg("-xf")
        .arg(&zip)
        .arg("-C")
        .arg(&unpacked)
        .status()
        .map_err(|e| format!("Extraction failed ({e})"))?;
    if !status.success() {
        return Err("Extraction failed".into());
    }
    let has_programs = |d: &std::path::Path| PROGRAMS.iter().all(|p| d.join(p).is_file());
    let from = std::iter::once(unpacked.clone())
        .chain(std::fs::read_dir(&unpacked).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()))
        .find(|d| has_programs(d))
        .ok_or("The package doesn't contain both programs")?;
    swap_programs(&from, install_dir)
}

/// Puts the new programs in place of the running ones: each old .exe is renamed to .old first (allowed while it
/// runs), and on any failure the renamed ones are put back.
#[cfg(windows)]
pub fn swap_programs(from: &std::path::Path, install_dir: &std::path::Path) -> Result<(), String> {
    let mut renamed: Vec<&str> = Vec::new();
    let undo = |renamed: &[&str]| {
        for name in renamed {
            let current = install_dir.join(name);
            let _ = std::fs::remove_file(&current);
            let _ = std::fs::rename(install_dir.join(format!("{name}.old")), &current);
        }
    };
    for name in PROGRAMS {
        let current = install_dir.join(name);
        let old = install_dir.join(format!("{name}.old"));
        let _ = std::fs::remove_file(&old);
        if current.exists() {
            if let Err(e) = std::fs::rename(&current, &old) {
                undo(&renamed);
                return Err(format!("can't move {name} aside ({e}); nothing was installed"));
            }
            renamed.push(name);
        }
        if let Err(e) = std::fs::copy(from.join(name), &current) {
            if !renamed.contains(&name) {
                let _ = std::fs::remove_file(&current);
            }
            undo(&renamed);
            return Err(format!("can't put the new {name} in place ({e}); nothing was installed"));
        }
    }
    Ok(())
}

/// Removes the `.old` programs a previous update left next to this one (they were still running then).
#[cfg(windows)]
pub fn remove_leftovers() {
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)) {
        for name in PROGRAMS {
            let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
        }
    }
}

/// Waits (up to `limit`) for the old service to exit, so it has freed the socket and the mixer's port.
#[cfg(windows)]
#[expect(unsafe_code, reason = "waiting for a process is a raw Win32 call")]
pub fn wait_for_exit(pid: u32, limit: std::time::Duration) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    // SAFETY: a wait-only handle, closed below
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else { return };
    // SAFETY: the handle opened above
    unsafe {
        WaitForSingleObject(process, limit.as_millis().try_into().unwrap_or(u32::MAX));
    }
    // SAFETY: the same handle, not used again
    let _ = unsafe { CloseHandle(process) };
}

#[cfg(not(windows))]
pub fn apply_update(release: &ReleaseInfo) -> Result<(), String> {
    let checksum_url =
        release.checksum_url.as_deref().ok_or("This release has no checksum, so it won't be installed")?;
    let dir = private_dir()?;
    let result = install_from(&dir, &release.package_url, checksum_url);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(not(windows))]
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
        use std::os::unix::fs::PermissionsExt as _;
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

    #[test]
    #[cfg(not(windows))]
    fn packaged_copies_leave_updates_to_the_package_manager() {
        assert!(is_system_install(std::path::Path::new("/usr/bin/cranked-audiod")));
        assert!(!is_system_install(std::path::Path::new("/home/someone/.local/bin/cranked-audiod")));
        assert!(!is_system_install(std::path::Path::new("/usrlocal/cranked-audiod")), "only the /usr folder itself");
        assert!(can_install(), "the test program runs from target/, which is not a system install");
    }

    #[cfg(windows)]
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cranked-swap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(windows)]
    #[test]
    fn swap_puts_new_programs_in_place_and_keeps_the_old_ones_aside() {
        let (new, installed) = (scratch("new"), scratch("installed"));
        for name in PROGRAMS {
            std::fs::write(new.join(name), "new").unwrap();
            std::fs::write(installed.join(name), "old").unwrap();
        }
        swap_programs(&new, &installed).unwrap();
        for name in PROGRAMS {
            assert_eq!(std::fs::read_to_string(installed.join(name)).unwrap(), "new");
            assert_eq!(std::fs::read_to_string(installed.join(format!("{name}.old"))).unwrap(), "old");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_swap_puts_everything_back() {
        let (new, installed) = (scratch("partial"), scratch("kept"));
        std::fs::write(new.join(PROGRAMS[0]), "new").unwrap(); // the second program is missing
        for name in PROGRAMS {
            std::fs::write(installed.join(name), "old").unwrap();
        }
        assert!(swap_programs(&new, &installed).is_err());
        for name in PROGRAMS {
            assert_eq!(std::fs::read_to_string(installed.join(name)).unwrap(), "old");
            assert!(!installed.join(format!("{name}.old")).exists());
        }
    }
}
