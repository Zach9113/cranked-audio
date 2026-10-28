//! Windows updates: a running .exe can be renamed but not overwritten, so the new programs are swapped in next to the
//! running ones and the service restarts into them.

#![expect(unsafe_code, reason = "waiting for the old service to exit is a raw Win32 call")]

use std::process::Command;
use std::sync::mpsc::Sender;

use crate::updater::{checksum_in, download, ReleaseInfo};
use crate::Event;

/// The Windows updater adapter: download the zip, check it, swap the two programs, restart into them.
pub struct ExeSwap;

impl crate::updater::Updater for ExeSwap {
    fn check(&self, events: Sender<Event>) {
        crate::platform::spawn_thread("updater-check", move || {
            // a release without a Windows build has nothing to update to here
            let result = crate::updater::check_for_update(PACKAGE_SUFFIX, |_| None);
            let _ = events.send(Event::UpdateCheckResult(result));
        });
    }

    fn can_install(&self) -> bool {
        true
    }

    fn install(&self, release: ReleaseInfo, events: Sender<Event>) {
        crate::platform::spawn_thread("updater-apply", move || {
            let _ = events.send(Event::UpdateApplyResult(apply_update(&release)));
        });
    }

    fn restarts_service(&self) -> bool {
        true
    }

    fn after_restart(&self, old_pid: Option<u32>) {
        // the old service may still hold the socket and the mixer's port for a moment
        if let Some(pid) = old_pid {
            wait_for_exit(pid, std::time::Duration::from_secs(15));
        }
        remove_leftovers();
    }
}

/// The release asset this platform installs.
const PACKAGE_SUFFIX: &str = "-windows-x86_64-experimental.zip";

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
const PROGRAMS: [&str; 2] = ["cranked-audiod.exe", "cranked-audio.exe"];

fn apply_update(release: &ReleaseInfo) -> Result<(), String> {
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
fn swap_programs(from: &std::path::Path, install_dir: &std::path::Path) -> Result<(), String> {
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
fn remove_leftovers() {
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)) {
        for name in PROGRAMS {
            let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
        }
    }
}

/// Waits (up to `limit`) for the old service to exit, so it has freed the socket and the mixer's port.
fn wait_for_exit(pid: u32, limit: std::time::Duration) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cranked-swap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
