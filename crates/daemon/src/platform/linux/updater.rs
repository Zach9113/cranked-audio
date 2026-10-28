//! Linux updates: the release tarball's install.sh puts the new programs in ~/.local/bin and restarts the service.

use std::process::Command;
use std::sync::mpsc::Sender;

use crate::updater::{checksum_in, download, ReleaseInfo, REPO};
use crate::Event;

/// The Linux updater adapter: download the tarball, check it, run its install.sh.
pub struct InstallScript;

impl crate::updater::Updater for InstallScript {
    fn check(&self, events: Sender<Event>) {
        crate::platform::spawn_thread("updater-check", move || {
            let fallback = |tag: &str| {
                Some(format!("https://github.com/{REPO}/releases/download/{tag}/cranked-audio-{tag}-x86_64.tar.gz"))
            };
            let _ = events.send(Event::UpdateCheckResult(crate::updater::check_for_update(PACKAGE_SUFFIX, fallback)));
        });
    }

    fn can_install(&self) -> bool {
        can_install()
    }

    fn install(&self, release: ReleaseInfo, events: Sender<Event>) {
        crate::platform::spawn_thread("updater-apply", move || {
            let _ = events.send(Event::UpdateApplyResult(apply_update(&release)));
        });
    }

    fn restarts_service(&self) -> bool {
        false
    }

    fn after_restart(&self, _old_pid: Option<u32>) {}
}

/// The release asset this platform installs.
const PACKAGE_SUFFIX: &str = "-x86_64.tar.gz";

/// Whether this copy can install updates itself. A copy a package manager put in /usr can't: the release's
/// installer would add a second copy in ~/.local/bin, so the package manager updates it instead.
fn can_install() -> bool {
    std::env::current_exe().is_ok_and(|program| !is_system_install(&program))
}

/// Whether a program lives in the system's own folders (installed by a package manager, e.g. from the AUR).
fn is_system_install(program: &std::path::Path) -> bool {
    program.starts_with("/usr")
}

/// A fresh folder only this user can open (never in a shared place like /tmp, never one that already existed),
/// so nothing else on the machine can swap the installer between unpacking and running it.
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

fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    let output = Command::new("sha256sum").arg(path).output().map_err(|e| format!("sha256sum: {e}"))?;
    checksum_in(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| "sha256sum gave no checksum".to_string())
}

fn apply_update(release: &ReleaseInfo) -> Result<(), String> {
    let checksum_url =
        release.checksum_url.as_deref().ok_or("This release has no checksum, so it won't be installed")?;
    let dir = private_dir()?;
    let result = install_from(&dir, &release.package_url, checksum_url);
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
    fn packaged_copies_leave_updates_to_the_package_manager() {
        assert!(is_system_install(std::path::Path::new("/usr/bin/cranked-audiod")));
        assert!(!is_system_install(std::path::Path::new("/home/someone/.local/bin/cranked-audiod")));
        assert!(!is_system_install(std::path::Path::new("/usrlocal/cranked-audiod")), "only the /usr folder itself");
        assert!(can_install(), "the test program runs from target/, which is not a system install");
    }
}
