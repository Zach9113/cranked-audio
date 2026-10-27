//! Windows global shortcuts via `RegisterHotKey`, configured in `config.toml` under `[shortcuts]`
//! (e.g. `"knob-1-up" = "Ctrl+Alt+Up"`, `"knob-1-button" = "Ctrl+Alt+M"`, `"open-window" = "Ctrl+Alt+C"`).
//! Button shortcuts track key release via `GetAsyncKeyState` so short tap = press action and long hold = hold action.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR,
    VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer, TranslateMessage, MSG, WM_APP, WM_HOTKEY,
    WM_TIMER,
};

use super::Shortcut;
use crate::Event;

const RELOAD: u32 = WM_APP + 1;

/// Parses a shortcut string like `"Ctrl+Alt+Up"`, `"Ctrl+Shift+F1"`, `"Alt+M"` into `(modifiers, vk)`.
pub fn parse_hotkey(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut key_part: Option<&str> = None;
    for raw in spec.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        match raw.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            _ => {
                if key_part.is_some() {
                    return None;
                }
                key_part = Some(raw);
            }
        }
    }
    let key = key_part?;
    let vk = parse_vk(key)?;
    Some((mods, vk))
}

fn parse_vk(key: &str) -> Option<u32> {
    let upper = key.to_ascii_uppercase();
    if upper.len() == 1 {
        let b = upper.as_bytes()[0];
        if b.is_ascii_uppercase() || b.is_ascii_digit() {
            return Some(u32::from(b));
        }
    }
    if let Some(rest) = upper.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + (n - 1));
            }
        }
    }
    match upper.as_str() {
        "UP" | "ARROWUP" => Some(u32::from(VK_UP.0)),
        "DOWN" | "ARROWDOWN" => Some(u32::from(VK_DOWN.0)),
        "LEFT" | "ARROWLEFT" => Some(u32::from(VK_LEFT.0)),
        "RIGHT" | "ARROWRIGHT" => Some(u32::from(VK_RIGHT.0)),
        "PGUP" | "PAGEUP" => Some(u32::from(VK_PRIOR.0)),
        "PGDN" | "PAGEDOWN" => Some(u32::from(VK_NEXT.0)),
        "HOME" => Some(u32::from(VK_HOME.0)),
        "END" => Some(u32::from(VK_END.0)),
        "INS" | "INSERT" => Some(u32::from(VK_INSERT.0)),
        "DEL" | "DELETE" => Some(u32::from(VK_DELETE.0)),
        "SPACE" => Some(u32::from(VK_SPACE.0)),
        "TAB" => Some(u32::from(VK_TAB.0)),
        "ENTER" | "RETURN" => Some(u32::from(VK_RETURN.0)),
        "ESC" | "ESCAPE" => Some(u32::from(VK_ESCAPE.0)),
        "BACKSPACE" | "BS" => Some(u32::from(VK_BACK.0)),
        "PLUS" | "=" | "OEM_PLUS" => Some(0xBB),
        "MINUS" | "-" | "OEM_MINUS" => Some(0xBD),
        _ => None,
    }
}

pub struct Shortcuts {
    bindings: Arc<Mutex<BTreeMap<String, String>>>,
    thread: u32,
}

impl Shortcuts {
    pub fn start(events: Sender<Event>, _knobs: usize, initial: &BTreeMap<String, String>) -> Option<Shortcuts> {
        let bindings = Arc::new(Mutex::new(initial.clone()));
        let for_thread = Arc::clone(&bindings);
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("shortcuts".into())
            .spawn(move || {
                crate::desktop::attach_input_desktop();
                let tid = unsafe { GetCurrentThreadId() };
                let _ = tx.send(tid);
                run_loop(events, for_thread);
            })
            .ok()?;
        let thread = rx.recv().ok()?;
        Some(Shortcuts { bindings, thread })
    }

    pub fn ensure(&mut self, _knobs: usize, updated: &BTreeMap<String, String>) {
        if let Ok(mut guard) = self.bindings.lock() {
            if *guard == *updated {
                return;
            }
            *guard = updated.clone();
        }
        unsafe {
            let _ = PostThreadMessageW(self.thread, RELOAD, WPARAM(0), LPARAM(0));
        }
    }
}

fn apply_registrations(
    bindings: &BTreeMap<String, String>,
    active: &mut HashMap<i32, (Shortcut, u32)>,
    held_buttons: &mut HashMap<usize, u32>,
) {
    for &id in active.keys() {
        unsafe {
            let _ = UnregisterHotKey(None, id);
        }
    }
    active.clear();
    held_buttons.clear();

    let mut next_id = 1i32;
    for (action_id, key_spec) in bindings {
        if key_spec.trim().is_empty() {
            continue;
        }
        let Some(shortcut) = Shortcut::from_id(action_id) else {
            eprintln!("shortcuts: unknown action {action_id:?}");
            continue;
        };
        let Some((mut mods, vk)) = parse_hotkey(key_spec) else {
            eprintln!("shortcuts: invalid key combination {key_spec:?} for {action_id}");
            continue;
        };
        // Non-repeating actions use MOD_NOREPEAT so holding the key doesn't re-trigger
        if matches!(shortcut, Shortcut::Button(_) | Shortcut::NextProfile | Shortcut::OpenWindow) {
            mods |= MOD_NOREPEAT;
        }
        let id = next_id;
        next_id += 1;
        if let Err(e) = unsafe { RegisterHotKey(None, id, mods, vk) } {
            eprintln!("shortcuts: couldn't register {key_spec} for {action_id} ({e})");
            continue;
        }
        active.insert(id, (shortcut, vk));
    }
}

fn run_loop(events: Sender<Event>, bindings: Arc<Mutex<BTreeMap<String, String>>>) {
    let mut active: HashMap<i32, (Shortcut, u32)> = HashMap::new();
    let mut held_buttons: HashMap<usize, u32> = HashMap::new();
    let mut poll_timer: usize = 0;

    if let Ok(initial) = bindings.lock() {
        apply_registrations(&initial, &mut active, &mut held_buttons);
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.hwnd.is_invalid() && msg.message == RELOAD {
            if let Ok(latest) = bindings.lock() {
                apply_registrations(&latest, &mut active, &mut held_buttons);
            }
            continue;
        }
        if msg.message == WM_HOTKEY {
            let id = msg.wParam.0 as i32;
            if let Some(&(shortcut, vk)) = active.get(&id) {
                match shortcut {
                    Shortcut::Button(knob) => {
                        if let std::collections::hash_map::Entry::Vacant(e) = held_buttons.entry(knob) {
                            e.insert(vk);
                            if let Some(req) = shortcut.request(Some(true)) {
                                let _ = events.send(Event::Remote(req));
                            }
                            if poll_timer == 0 {
                                poll_timer = unsafe { SetTimer(None, 0, 40, None) };
                            }
                        }
                    }
                    other => {
                        if let Some(req) = other.request(Some(true)) {
                            let _ = events.send(Event::Remote(req));
                        }
                    }
                }
            }
            continue;
        }
        if msg.message == WM_TIMER && !held_buttons.is_empty() {
            held_buttons.retain(|&knob, &mut vk| {
                let still_down = unsafe { GetAsyncKeyState(vk as i32) } < 0;
                if !still_down {
                    if let Some(req) = Shortcut::Button(knob).request(Some(false)) {
                        let _ = events.send(Event::Remote(req));
                    }
                }
                still_down
            });
            if held_buttons.is_empty() && poll_timer != 0 {
                unsafe {
                    let _ = KillTimer(None, poll_timer);
                }
                poll_timer = 0;
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifier_and_key_combinations() {
        assert_eq!(parse_hotkey("Ctrl+Alt+Up"), Some((MOD_CONTROL | MOD_ALT, u32::from(VK_UP.0))));
        assert_eq!(parse_hotkey("ctrl + shift + f1"), Some((MOD_CONTROL | MOD_SHIFT, 0x70)));
        assert_eq!(parse_hotkey("Alt+M"), Some((MOD_ALT, u32::from(b'M'))));
        assert_eq!(parse_hotkey("Win+PageDown"), Some((MOD_WIN, u32::from(VK_NEXT.0))));
        assert_eq!(parse_hotkey("F13"), Some((HOT_KEY_MODIFIERS(0), 0x7C)));
        assert_eq!(parse_hotkey("Ctrl+"), None);
        assert_eq!(parse_hotkey(""), None);
    }
}
