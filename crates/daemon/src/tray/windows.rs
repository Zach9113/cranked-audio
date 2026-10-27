//! Windows: a notification-area icon (through tray-icon) on its own thread, which runs the message loop the icon
//! and its menu need. Left click opens the window; right click shows the menu.

#![expect(unsafe_code, reason = "the Win32 message loop is raw FFI")]

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_APP,
};

use super::{Action, Tray, ICONS};

/// Posted to the tray thread when what the tray shows changed.
const REFRESH: u32 = WM_APP + 1;
const ICON_SIZE: i32 = 32;

/// What the main loop keeps to change the tray.
pub struct Handle {
    state: Arc<Mutex<(Tray, HashMap<MenuId, Action>)>>,
    thread: u32,
}

impl Handle {
    pub fn update(&self, change: impl FnOnce(&mut Tray)) {
        if let Ok(mut guard) = self.state.lock() {
            change(&mut guard.0);
        }
        // SAFETY: posting a plain message (no pointers) to a thread that runs a message loop
        let _ = unsafe { PostThreadMessageW(self.thread, REFRESH, WPARAM::default(), LPARAM::default()) };
    }
}

/// The menu for what the tray shows now, and which action each item stands for.
fn menu(tray: &Tray) -> (Menu, HashMap<MenuId, Action>) {
    let menu = Menu::new();
    let mut actions = HashMap::new();
    let open = MenuItem::new("Open Cranked Audio", true, None);
    actions.insert(open.id().clone(), Action::OpenWindow);
    let _ = menu.append(&open);
    if tray.profiles.len() > 1 {
        let _ = menu.append(&PredefinedMenuItem::separator());
        for profile in &tray.profiles {
            let item = CheckMenuItem::new(profile, true, *profile == tray.profile, None);
            actions.insert(item.id().clone(), Action::Profile(profile.clone()));
            let _ = menu.append(&item);
        }
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let edit = MenuItem::new("Edit settings file…", true, None);
    actions.insert(edit.id().clone(), Action::EditSettings);
    let quit = MenuItem::new("Quit", true, None);
    actions.insert(quit.id().clone(), Action::Quit);
    let _ = menu.append_items(&[&edit, &quit]);
    (menu, actions)
}

fn tooltip(tray: &Tray) -> String {
    let mixer = if tray.mixer { "Mixer connected" } else { "No mixer: on-screen controls" };
    format!("Cranked Audio\n{mixer}\nProfile: {}", tray.profile)
}

fn build(tray: &Tray) -> Result<(TrayIcon, HashMap<MenuId, Action>), String> {
    let rgba = ICONS.iter().find(|(size, _)| *size == ICON_SIZE).map(|(_, rgba)| rgba.to_vec()).unwrap_or_default();
    let icon = Icon::from_rgba(rgba, ICON_SIZE as u32, ICON_SIZE as u32).map_err(|e| e.to_string())?;
    let (menu, actions) = menu(tray);
    let icon = TrayIconBuilder::new()
        .with_id("cranked-audio")
        .with_icon(icon)
        .with_tooltip(tooltip(tray))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;
    Ok((icon, actions))
}

fn run(icon: &TrayIcon, state: &Arc<Mutex<(Tray, HashMap<MenuId, Action>)>>) {
    let mut message = MSG::default();
    // SAFETY: `message` outlives each call; GetMessageW returns 0 on WM_QUIT and -1 on errors
    while unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0 > 0 {
        if message.hwnd.is_invalid() && message.message == REFRESH {
            if let Ok(mut guard) = state.lock() {
                let (menu, new_actions) = menu(&guard.0);
                let tip = tooltip(&guard.0);
                guard.1 = new_actions;
                drop(guard);
                icon.set_menu(Some(Box::new(menu)));
                let _ = icon.set_tooltip(Some(tip));
            }
        }
        // SAFETY: a message GetMessageW just filled in
        let _ = unsafe { TranslateMessage(&raw const message) };
        // SAFETY: the same message
        unsafe {
            DispatchMessageW(&raw const message);
        }
    }
}

pub fn start(tray: Tray) -> Option<Handle> {
    let state = Arc::new(Mutex::new((tray, HashMap::new())));
    let for_menu = Arc::clone(&state);
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Ok(guard) = for_menu.lock() {
            if let Some(action) = guard.1.get(event.id()).cloned() {
                guard.0.send(action);
            }
        }
    }));
    let for_icon = Arc::clone(&state);
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            id: _,
            position: _,
            rect: _,
        }
        | TrayIconEvent::DoubleClick { button: MouseButton::Left, id: _, position: _, rect: _ } = event
        {
            if let Ok(guard) = for_icon.lock() {
                guard.0.send(Action::OpenWindow);
            }
        }
    }));

    let (ready, started) = mpsc::channel();
    let for_thread = Arc::clone(&state);
    let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || {
        crate::desktop::attach_input_desktop();
        let built = for_thread.lock().map_err(|e| e.to_string()).and_then(|mut guard| {
            let (icon, actions) = build(&guard.0)?;
            guard.1 = actions;
            Ok(icon)
        });
        match built {
            Ok(icon) => {
                // SAFETY: no arguments; the id of this thread, which owns the icon and runs its message loop
                let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
                run(&icon, &for_thread);
            }
            Err(e) => {
                eprintln!("tray: not available ({e})");
                let _ = ready.send(None);
            }
        }
    });
    if spawned.is_err() {
        return None;
    }
    let thread = started.recv().ok().flatten()?;
    Some(Handle { state, thread })
}
