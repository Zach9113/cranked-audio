//! Windows: media buttons control a player through Windows' media sessions, picked the way the Linux side picks an
//! MPRIS player (a media key is the fallback). No volume popups yet, and the screen counts as always on.

use std::sync::mpsc::{Receiver, Sender};

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};

use super::{Command, MediaKey};

fn tap(key: VIRTUAL_KEY) {
    let input = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: key, dwFlags: flags, ..Default::default() } },
    };
    let inputs = [input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)];
    // SAFETY: two fully set up keyboard inputs, with their real size
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        eprintln!("media: the key wasn't sent");
    }
}

/// Playing beats paused beats anything else; on a tie, the session Windows calls current wins.
fn rank(status: Option<Status>, current: bool) -> (u8, bool) {
    let by_status = match status {
        Some(Status::Playing) => 0,
        Some(Status::Paused) => 1,
        _ => 2,
    };
    (by_status, !current)
}

fn pick_session(manager: &SessionManager) -> windows::core::Result<Option<Session>> {
    let current = manager.GetCurrentSession().and_then(|s| s.SourceAppUserModelId()).ok();
    let sessions = manager.GetSessions()?;
    let mut best: Option<((u8, bool), Session)> = None;
    for i in 0..sessions.Size()? {
        let session = sessions.GetAt(i)?;
        let status = session.GetPlaybackInfo().and_then(|info| info.PlaybackStatus()).ok();
        let is_current = current.is_some() && session.SourceAppUserModelId().ok() == current;
        let key = rank(status, is_current);
        if best.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
            best = Some((key, session));
        }
    }
    Ok(best.map(|(_, session)| session))
}

/// Ok(false) when there's no media session to send it to.
fn send_to_session(key: MediaKey) -> windows::core::Result<bool> {
    let manager = SessionManager::RequestAsync()?.join()?;
    let Some(session) = pick_session(&manager)? else { return Ok(false) };
    match key {
        MediaKey::PlayPause => session.TryTogglePlayPauseAsync()?,
        MediaKey::Next => session.TrySkipNextAsync()?,
        MediaKey::Previous => session.TrySkipPreviousAsync()?,
    }
    .join()
}

/// Runs forever on its own thread.
pub fn run(commands: Receiver<Command>) {
    // SAFETY: once, on this thread, before any other COM call here
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    for command in commands {
        if let Command::Media(key) = command {
            match send_to_session(key) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => eprintln!("media: no session control ({e}), sending the key"),
            }
            tap(match key {
                MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
                MediaKey::Next => VK_MEDIA_NEXT_TRACK,
                MediaKey::Previous => VK_MEDIA_PREV_TRACK,
            });
        }
    }
}

/// Not watched on Windows yet: the LEDs stay on while the screen sleeps.
pub fn watch_screen(_events: Sender<crate::Event>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_beats_paused_beats_the_rest_and_current_breaks_ties() {
        let playing = rank(Some(Status::Playing), false);
        assert!(playing < rank(Some(Status::Paused), true));
        assert!(rank(Some(Status::Paused), false) < rank(Some(Status::Stopped), true));
        assert!(rank(None, false) == rank(Some(Status::Closed), false));
        assert!(rank(Some(Status::Playing), true) < playing);
    }
}
