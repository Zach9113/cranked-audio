//! Windows: media keys go to the active player the way a keyboard's media keys would (Windows picks the player).
//! No volume popups yet, and the screen counts as always on.

use std::sync::mpsc::{Receiver, Sender};

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

/// Runs forever on its own thread.
pub fn run(commands: Receiver<Command>) {
    for command in commands {
        if let Command::Media(key) = command {
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
