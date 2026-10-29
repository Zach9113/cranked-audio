//! Windows: keyboard knobs and rotary controllers (enumerated via Raw Input + HID product strings, captured via
//! `WH_KEYBOARD_LL` so volume/mute/media keys from the knob can be taken over without affecting typing), and serial
//! ports by USB vendor.

#![expect(unsafe_code, reason = "Raw Input, HID strings and the low-level keyboard hook are raw Win32 calls")]

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Devices::HumanInterfaceDevice::{HidD_GetManufacturerString, HidD_GetProductString};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_VOLUME_DOWN, VK_VOLUME_MUTE,
    VK_VOLUME_UP,
};
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO, RIDI_DEVICENAME,
    RID_DEVICE_INFO, RIM_TYPEHID, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, SetTimer, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_TIMER,
};

use super::{attach_input_desktop, CRANKED_EXTRA_INFO};
use crate::desktop::MediaKey;
use crate::devices::{is_board, send, Input, RETRY};
use crate::Event;

/// Substrings in device names that indicate virtual injectors, monitor audio jacks, microphones, coolers or system
/// buttons that shouldn't appear as knob candidates.
pub fn is_bad_name(name: &str) -> bool {
    const BAD: &[&str] = &[
        "cranked audio",
        "passthrough",
        "action injector",
        "virtual",
        "uinput",
        "ydotool",
        "xdotool",
        "monitor headset",
        "generic lg monitor",
        "ultragear monitor",
        "hid i2c",
        "hdmi/dp",
        "hda intel",
        "hda nvidia",
        "power button",
        "sleep button",
        "video bus",
        "pc speaker",
        "mystic light",
        "commander core",
        "lcd cap",
        "usb audio",
        "microphone",
        "webcam",
        "headset mic",
    ];
    let lower = name.to_lowercase();
    let realtek = lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        word.strip_prefix("alc").is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    });
    realtek || BAD.iter().any(|b| lower.contains(b))
}

fn hid_strings(device_path: &[u16]) -> Option<(String, String)> {
    // SAFETY: metadata-only open (desired access 0) on a device path returned by GetRawInputDeviceInfoW
    let handle = unsafe {
        CreateFileW(
            PCWSTR(device_path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()?;
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut m_buf = [0_u16; 128];
    let mut p_buf = [0_u16; 128];
    // both buffers are 256 bytes
    let buf_bytes = u32::try_from(std::mem::size_of_val(&m_buf)).unwrap_or(0);
    // SAFETY: the handle opened above and a buffer of `buf_bytes` bytes
    let m_ok = unsafe { HidD_GetManufacturerString(handle, m_buf.as_mut_ptr().cast::<core::ffi::c_void>(), buf_bytes) };
    // SAFETY: as above
    let p_ok = unsafe { HidD_GetProductString(handle, p_buf.as_mut_ptr().cast::<core::ffi::c_void>(), buf_bytes) };
    // SAFETY: the same handle, not used again
    let _ = unsafe { CloseHandle(handle) };

    let decode = |ok: bool, buf: &[u16]| -> String {
        if !ok {
            return String::new();
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len]).trim().to_string()
    };
    let manufacturer = decode(m_ok, &m_buf);
    let product = decode(p_ok, &p_buf);
    if manufacturer.is_empty() && product.is_empty() {
        None
    } else {
        Some((manufacturer, product))
    }
}

/// Formats a HID device's manufacturer + product + suffix to match Linux evdev naming
/// (e.g. `"BY Tech"` + `"Gaming Keyboard"` + `"Consumer Control"` -> `"BY Tech Gaming Keyboard Consumer Control"`).
pub fn format_hid_name(manufacturer: &str, product: &str, consumer_control: bool) -> String {
    let m = manufacturer.trim();
    let p = product.trim();
    let base = match (!m.is_empty(), !p.is_empty()) {
        (true, true) if p.to_lowercase().starts_with(&m.to_lowercase()) => p.to_string(),
        (true, true) => format!("{m} {p}"),
        (true, false) => m.to_string(),
        (false, true) => p.to_string(),
        (false, false) => return String::new(),
    };
    if consumer_control && !base.to_lowercase().ends_with("consumer control") {
        format!("{base} Consumer Control")
    } else {
        base
    }
}

/// Input devices that have volume keys or Consumer Control endpoints, by name, and which of them also type.
fn keyboard_knobs() -> (Vec<String>, Vec<String>) {
    let mut count = 0_u32;
    let list_size = u32::try_from(std::mem::size_of::<RAWINPUTDEVICELIST>()).unwrap_or(0);
    // SAFETY: asking only for the count, into a local
    if unsafe { GetRawInputDeviceList(None, &raw mut count, list_size) } != 0 || count == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    // SAFETY: a buffer of `count` entries, as Windows just said it needs
    let fetched = unsafe { GetRawInputDeviceList(Some(devices.as_mut_ptr()), &raw mut count, list_size) };
    if fetched == u32::MAX {
        return (Vec::new(), Vec::new());
    }
    devices.truncate(fetched as usize);

    let mut found: Vec<(bool, String)> = Vec::new();
    for dev in devices {
        let info_bytes = u32::try_from(std::mem::size_of::<RID_DEVICE_INFO>()).unwrap_or(0);
        let mut info = RID_DEVICE_INFO { cbSize: info_bytes, ..Default::default() };
        let mut info_size = info.cbSize;
        // SAFETY: a device handle from the list above and a RID_DEVICE_INFO of the size we pass
        let ok = unsafe {
            GetRawInputDeviceInfoW(
                Some(dev.hDevice),
                RIDI_DEVICEINFO,
                Some((&raw mut info).cast::<core::ffi::c_void>()),
                &raw mut info_size,
            )
        };
        if ok == u32::MAX {
            continue;
        }
        let (is_typing, is_consumer) = match dev.dwType {
            RIM_TYPEKEYBOARD => (true, false),
            RIM_TYPEHID => {
                // SAFETY: for RIM_TYPEHID devices Windows fills in the `hid` member of the union
                let hid = unsafe { info.Anonymous.hid };
                // UsagePage 0x000C Usage 0x0001 = Consumer Control (volume wheel / media keys)
                if hid.usUsagePage == 0x000C && hid.usUsage == 0x0001 {
                    (false, true)
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        let mut name_len = 0_u32;
        // SAFETY: asking only for the name's length, into a local
        let _ = unsafe { GetRawInputDeviceInfoW(Some(dev.hDevice), RIDI_DEVICENAME, None, &raw mut name_len) };
        if name_len == 0 {
            continue;
        }
        let mut path = vec![0_u16; usize::try_from(name_len).unwrap_or(0) + 1];
        // SAFETY: a buffer one character longer than the name length Windows just gave
        let written = unsafe {
            GetRawInputDeviceInfoW(
                Some(dev.hDevice),
                RIDI_DEVICENAME,
                Some(path.as_mut_ptr().cast::<core::ffi::c_void>()),
                &raw mut name_len,
            )
        };
        if written == u32::MAX {
            continue;
        }
        let Some((manufacturer, product)) = hid_strings(&path) else { continue };
        let friendly = format_hid_name(&manufacturer, &product, is_consumer);
        if friendly.is_empty() || is_bad_name(&friendly) {
            continue;
        }
        found.push((is_typing, friendly));
    }

    found.sort();

    let mut seen = HashSet::new();
    let mut names = Vec::new();
    let mut non_typing = HashSet::new();
    let mut typing_candidates = HashSet::new();

    for (is_typing, name) in found {
        if is_typing {
            typing_candidates.insert(name.clone());
        } else {
            non_typing.insert(name.clone());
        }
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    let typing: Vec<String> =
        names.iter().filter(|n| typing_candidates.contains(*n) && !non_typing.contains(*n)).cloned().collect();

    (names, typing)
}

/// Maps a Windows virtual key (`VK_VOLUME_UP`, `VK_VOLUME_DOWN`, `VK_VOLUME_MUTE`, `VK_MEDIA_*`) to a knob `Input`.
/// Returns `(Option<Input>, bool)` where the second element is whether the hook should swallow the key event.
pub fn vk_input(
    vk: u16,
    down: bool,
    mute_already_down: &mut bool,
    knob: usize,
    step: u8,
    grab: bool,
) -> (Option<Input>, bool) {
    let step = i32::from(step.max(1));
    match VIRTUAL_KEY(vk) {
        VK_VOLUME_UP => (down.then_some(Input::Step { knob, delta: step }), grab),
        VK_VOLUME_DOWN => (down.then_some(Input::Step { knob, delta: -step }), grab),
        VK_VOLUME_MUTE => {
            if down {
                if *mute_already_down {
                    (None, grab)
                } else {
                    *mute_already_down = true;
                    (Some(Input::Button { knob, down: true }), grab)
                }
            } else {
                *mute_already_down = false;
                (Some(Input::Button { knob, down: false }), grab)
            }
        }
        VK_MEDIA_PLAY_PAUSE if grab => (down.then_some(Input::Media(MediaKey::PlayPause)), true),
        VK_MEDIA_NEXT_TRACK if grab => (down.then_some(Input::Media(MediaKey::Next)), true),
        VK_MEDIA_PREV_TRACK if grab => (down.then_some(Input::Media(MediaKey::Previous)), true),
        _ => (None, false),
    }
}

struct HookContext {
    knob: usize,
    step: u8,
    grab: bool,
    mute_down: bool,
    events: Sender<Event>,
}

static HOOK_CTX: Mutex<Option<HookContext>> = Mutex::new(None);

unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        // SAFETY: for WH_KEYBOARD_LL with code >= 0, lparam points to a KBDLLHOOKSTRUCT (checked non-null above)
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore synthetic media key events sent by our own fallback in `desktop/windows.rs`, while still allowing
        // hardware/driver consumer-control events even if a HID filter sets `LLKHF_INJECTED`.
        if kb.dwExtraInfo != CRANKED_EXTRA_INFO {
            // keyboard message ids are small; anything that doesn't fit isn't one
            let msg = u32::try_from(wparam.0).unwrap_or(0);
            let down = match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            if let Some(down) = down {
                if let Ok(mut guard) = HOOK_CTX.lock() {
                    if let Some(ctx) = guard.as_mut() {
                        let (input, swallow) =
                            vk_input(vk_code(kb.vkCode), down, &mut ctx.mute_down, ctx.knob, ctx.step, ctx.grab);
                        if let Some(input) = input {
                            let _ = send(&ctx.events, input);
                        }
                        if swallow {
                            return LRESULT(1);
                        }
                    }
                }
            }
        }
    }
    // SAFETY: passing the event on unchanged, with the arguments Windows gave us
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Virtual-key codes are 1..=254; anything else isn't a key we handle.
fn vk_code(code: u32) -> u16 {
    u16::try_from(code).unwrap_or(0)
}

fn device_present(target: &str) -> bool {
    let (names, _) = keyboard_knobs();
    let target_base = target.trim().trim_end_matches(" Consumer Control").trim_end_matches(" consumer control");
    names.iter().any(|n| {
        n.eq_ignore_ascii_case(target) || n.trim_end_matches(" Consumer Control").eq_ignore_ascii_case(target_base)
    })
}

/// The Windows input hardware adapter: keyboard knobs through Raw Input and a low-level keyboard hook, COM ports.
pub struct RawInput;

impl crate::devices::InputHardware for RawInput {
    fn keyboard_knobs(&self) -> (Vec<String>, Vec<String>) {
        keyboard_knobs()
    }

    fn serial_ports(&self) -> Vec<String> {
        serial_ports()
    }

    fn keyboard_knob(&self, name: String, knob: usize, step: u8, grab: bool) -> Box<dyn crate::devices::InputSource> {
        Box::new(RawInputKnob { name, knob, step, grab })
    }

    fn keyboard_access(&self) -> Option<cranked_ipc::AccessProblem> {
        // Raw Input reads every keyboard without extra permissions
        None
    }
}

/// A keyboard knob read through Raw Input (with `grab`, the hook keeps its volume keys from Windows).
struct RawInputKnob {
    name: String,
    knob: usize,
    step: u8,
    grab: bool,
}

impl crate::devices::InputSource for RawInputKnob {
    fn run(&self, events: &Sender<Event>, current: &dyn Fn() -> bool) {
        keyboard(&self.name, self.knob, self.step, self.grab, events, current);
    }
}

fn keyboard(name: &str, knob: usize, step: u8, grab: bool, events: &Sender<Event>, current: &dyn Fn() -> bool) {
    attach_input_desktop();
    let knob = knob.saturating_sub(1);
    let mut reported = false;
    while current() {
        if !device_present(name) {
            if !reported {
                eprintln!("keyboard knob: {name:?} not found; waiting for it");
                reported = true;
            }
            std::thread::sleep(RETRY);
            continue;
        }
        reported = false;
        if let Ok(mut guard) = HOOK_CTX.lock() {
            *guard = Some(HookContext { knob, step, grab, mute_down: false, events: events.clone() });
        }
        // SAFETY: this program's own module handle
        let hmod = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
        // SAFETY: a low-level hook (runs on this thread, which pumps messages below) with a callback that lives for
        // the whole program; it is removed again before the loop starts over
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), hmod, 0) };
        let Ok(hook) = hook else {
            eprintln!("keyboard knob: couldn't install keyboard hook for {name:?}");
            std::thread::sleep(RETRY);
            continue;
        };
        eprintln!("keyboard knob: {name} on knob {}{}", knob + 1, if grab { ", taken over" } else { "" });
        // Keep the message loop completely non-blocking so Windows never times out `LowLevelHooksTimeout`.
        // SAFETY: a thread timer (no window, no callback): it arrives as WM_TIMER here
        let timer = unsafe { SetTimer(None, 0, 500, None) };
        let mut msg = MSG::default();
        // SAFETY: `msg` outlives each call; GetMessageW returns 0 on WM_QUIT and -1 on errors
        while current() && unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.0 > 0 {
            if msg.message == WM_TIMER && !current() {
                break;
            }
            // SAFETY: a message GetMessageW just filled in
            let _ = unsafe { TranslateMessage(&raw const msg) };
            // SAFETY: the same message
            unsafe {
                DispatchMessageW(&raw const msg);
            }
        }
        // SAFETY: the timer started above
        let _ = unsafe { KillTimer(None, timer) };
        // SAFETY: the hook installed above, not used again
        let _ = unsafe { UnhookWindowsHookEx(hook) };
        if let Ok(mut guard) = HOOK_CTX.lock() {
            *guard = None;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Serial ports that could be a deej box: USB boards only, and not the mixer.
fn serial_ports() -> Vec<String> {
    let Ok(ports) = serialport::available_ports() else { return Vec::new() };
    let mut ports: Vec<String> = ports
        .into_iter()
        .filter(|port| match &port.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                is_board(&format!("{:04x}", info.vid)) && !crate::serial::is_mixer(info.vid, info.pid)
            }
            _ => false,
        })
        .map(|port| port.port_name)
        .collect();
    ports.sort();
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_names_match_linux_evdev_names() {
        assert_eq!(format_hid_name("BY Tech", "Gaming Keyboard", true), "BY Tech Gaming Keyboard Consumer Control");
        assert_eq!(format_hid_name("BY Tech", "Gaming Keyboard", false), "BY Tech Gaming Keyboard");
        assert_eq!(format_hid_name("Logitech", "USB Receiver", true), "Logitech USB Receiver Consumer Control");
        assert_eq!(format_hid_name("Corsair", "CORSAIR iCUE Commander CORE", false), "CORSAIR iCUE Commander CORE");
    }

    #[test]
    fn bad_device_names_are_filtered_out() {
        assert!(is_bad_name("Generic LG Monitor Headset MIC Consumer Control"));
        assert!(is_bad_name("FIFINE Microphones FIFINE K658 Microphone Consumer Control"));
        assert!(is_bad_name("CORSAIR iCUE Commander CORE"));
        assert!(is_bad_name("MSI MYSTIC LIGHT"));
        assert!(!is_bad_name("BY Tech Gaming Keyboard Consumer Control"));
        assert!(!is_bad_name("BY Tech Gaming Keyboard"));
        assert!(!is_bad_name("Logitech USB Receiver Consumer Control"));
    }

    #[test]
    fn volume_and_mute_keys_map_and_swallow_when_grabbed() {
        let mut mute_down = false;
        assert_eq!(
            vk_input(VK_VOLUME_UP.0, true, &mut mute_down, 5, 2, true),
            (Some(Input::Step { knob: 5, delta: 2 }), true)
        );
        assert_eq!(
            vk_input(VK_VOLUME_DOWN.0, true, &mut mute_down, 5, 2, false),
            (Some(Input::Step { knob: 5, delta: -2 }), false)
        );
        assert_eq!(vk_input(VK_VOLUME_UP.0, false, &mut mute_down, 5, 2, true), (None, true));
        // First mute keydown triggers Button down: true; auto-repeats while held do not re-trigger
        assert_eq!(
            vk_input(VK_VOLUME_MUTE.0, true, &mut mute_down, 5, 2, true),
            (Some(Input::Button { knob: 5, down: true }), true)
        );
        assert_eq!(vk_input(VK_VOLUME_MUTE.0, true, &mut mute_down, 5, 2, true), (None, true));
        assert_eq!(
            vk_input(VK_VOLUME_MUTE.0, false, &mut mute_down, 5, 2, true),
            (Some(Input::Button { knob: 5, down: false }), true)
        );
        assert_eq!(
            vk_input(VK_MEDIA_PLAY_PAUSE.0, true, &mut mute_down, 5, 2, true),
            (Some(Input::Media(MediaKey::PlayPause)), true)
        );
        assert_eq!(vk_input(VK_MEDIA_PLAY_PAUSE.0, true, &mut mute_down, 5, 2, false), (None, false));
    }
}
