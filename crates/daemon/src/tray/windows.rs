//! Windows: a notification-area icon (through tray-icon) on its own thread, which runs the message loop the icon
//! and its menu need. Left click opens the window; right click shows the menu.

#![expect(unsafe_code, reason = "the Win32 message loop is raw FFI")]

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
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
    tray: Arc<Mutex<Tray>>,
    thread: u32,
}

impl Handle {
    pub fn update(&self, change: impl FnOnce(&mut Tray)) {
        if let Ok(mut tray) = self.tray.lock() {
            change(&mut tray);
        }
        // SAFETY: posting a plain message (no pointers) to a thread that runs a message loop
        let _ = unsafe { PostThreadMessageW(self.thread, REFRESH, Default::default(), Default::default()) };
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

fn run(icon: &TrayIcon, mut actions: HashMap<MenuId, Action>, tray: &Mutex<Tray>) {
    let mut message = MSG::default();
    // SAFETY: `message` outlives each call; GetMessageW returns 0 on WM_QUIT and -1 on errors
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        if message.hwnd.is_invalid() && message.message == REFRESH {
            if let Ok(tray) = tray.lock() {
                let (menu, new_actions) = menu(&tray);
                icon.set_menu(Some(Box::new(menu)));
                actions = new_actions;
                let _ = icon.set_tooltip(Some(tooltip(&tray)));
            }
        }
        // SAFETY: a message GetMessageW just filled in
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let Ok(tray) = tray.lock() else { continue };
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = actions.get(event.id()) {
                tray.send(action.clone());
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                tray.send(Action::OpenWindow);
            }
        }
    }
}

pub fn start(tray: Tray) -> Option<Handle> {
    let tray = Arc::new(Mutex::new(tray));
    let (ready, started) = mpsc::channel();
    let for_thread = Arc::clone(&tray);
    let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || {
        let built = for_thread.lock().map_err(|e| e.to_string()).and_then(|t| build(&t));
        match built {
            Ok((icon, actions)) => {
                // SAFETY: no arguments; the id of this thread, which owns the icon and runs its message loop
                let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
                run(&icon, actions, &for_thread);
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
    Some(Handle { tray, thread })
}
