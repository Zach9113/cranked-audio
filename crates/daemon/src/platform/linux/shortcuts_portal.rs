//! Global shortcuts through the XDG desktop portal (`org.freedesktop.portal.GlobalShortcuts`), for desktops without
//! KDE's kglobalaccel that offer it (Hyprland, and others as they add it). The desktop asks the user which keys to
//! use, once; after that, presses arrive as `Activated` / `Deactivated` signals on the portal session.
//!
//! The portal sends no key repeats: holding a turn shortcut turns the knob one step. A button shortcut still taps
//! for the press action and holds for the hold action.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::shortcuts::{all, Shortcut};
use crate::Event;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const GLOBAL_SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
/// The name the portal shows for us (host apps register it; sandboxed ones get it from their sandbox).
const APP_ID: &str = "io.github.zach9113.CrankedAudio";

/// The open portal session: the connection it lives on and its handle (it closes with the connection).
type Session = (Connection, OwnedObjectPath);

/// The shortcuts adapter for desktops with the `GlobalShortcuts` portal.
#[derive(Default)]
pub struct Portal {
    events: Option<Sender<Event>>,
    registered: usize,
    session: Arc<Mutex<Option<Session>>>,
}

impl crate::shortcuts::GlobalShortcuts for Portal {
    fn start(&mut self, events: Sender<Event>, knobs: usize, _bindings: &BTreeMap<String, String>) {
        self.registered = knobs;
        self.events = Some(events.clone());
        self.open(events, false);
    }

    /// More knobs need their shortcuts too: a fresh session binds the whole set (the desktop asks again).
    fn ensure(&mut self, knobs: usize, _bindings: &BTreeMap<String, String>) {
        let Some(events) = self.events.clone() else { return };
        if knobs <= self.registered {
            return;
        }
        self.registered = knobs;
        let had_session = if let Some((connection, session)) = self.session.lock().ok().and_then(|mut open| open.take())
        {
            let _ = connection.call_method(
                Some(PORTAL),
                session.as_str(),
                Some("org.freedesktop.portal.Session"),
                "Close",
                &(),
            );
            true
        } else {
            false
        };
        if had_session || !is_gnome() {
            self.open(events, false);
        }
    }

    fn configure(&mut self) {
        let already_bound = self.session.lock().is_ok_and(|open| open.is_some());
        if already_bound && is_gnome() {
            crate::service::spawn_detached(
                std::path::Path::new("gnome-control-center"),
                &["applications".as_ref(), APP_ID.as_ref()],
            );
            return;
        }
        if !already_bound {
            if let Some(events) = self.events.clone() {
                self.open(events, true);
            }
        }
    }
}

impl Portal {
    /// Binds the shortcuts and listens for them on a thread of its own: the desktop's dialog can take a while.
    fn open(&self, events: Sender<Event>, prompt: bool) {
        let (knobs, slot) = (self.registered, Arc::clone(&self.session));
        crate::platform::spawn_thread("shortcuts", move || {
            if let Err(e) = bind_and_listen(&events, knobs, &slot, prompt) {
                eprintln!("shortcuts: the desktop's shortcut portal isn't available ({e})");
            }
        });
    }
}

fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.to_ascii_uppercase().split(':').any(|part| part == "GNOME"))
}

fn has_bound_shortcuts(connection: &Connection, session: &OwnedObjectPath) -> bool {
    let options = HashMap::from([("handle_token", Value::from("cranked_list"))]);
    let Ok(listed) = request(connection, "cranked_list", "ListShortcuts", &(session, options)) else {
        return false;
    };
    take::<Vec<(String, HashMap<String, OwnedValue>)>>(&listed, "shortcuts")
        .is_ok_and(|shortcuts| !shortcuts.is_empty())
}

fn bind_and_listen(
    events: &Sender<Event>,
    knobs: usize,
    slot: &Mutex<Option<Session>>,
    prompt: bool,
) -> zbus::Result<()> {
    let connection = Connection::session()?;
    // name ourselves first, so the desktop's dialog and settings say "Cranked Audio" (an older portal lacks this)
    let no_options: HashMap<&str, Value<'_>> = HashMap::new();
    let _ = connection.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some("org.freedesktop.host.portal.Registry"),
        "Register",
        &(APP_ID, &no_options),
    );

    let options = HashMap::from([
        ("handle_token", Value::from("cranked_create")),
        ("session_handle_token", Value::from("cranked_shortcuts")),
    ]);
    let created = request(&connection, "cranked_create", "CreateSession", &(options,))?;
    let session_handle: String = take(&created, "session_handle")?;
    let session = OwnedObjectPath::try_from(session_handle).map_err(zbus::Error::from)?;

    // On GNOME, BindShortcuts pops up a modal dialog when no shortcuts are saved yet; wait until the user clicks
    // "Set shortcuts…" before prompting for the first time.
    if !prompt && is_gnome() && !has_bound_shortcuts(&connection, &session) {
        let _ = connection.call_method(
            Some(PORTAL),
            session.as_str(),
            Some("org.freedesktop.portal.Session"),
            "Close",
            &(),
        );
        return Ok(());
    }

    let shortcuts: Vec<(String, HashMap<&str, Value<'_>>)> = all(knobs)
        .into_iter()
        .map(|shortcut| (shortcut.id(), HashMap::from([("description", Value::from(shortcut.friendly()))])))
        .collect();
    let options = HashMap::from([("handle_token", Value::from("cranked_bind"))]);
    request(&connection, "cranked_bind", "BindShortcuts", &(&session, shortcuts, "", options))?;
    if let Ok(mut open) = slot.lock() {
        *open = Some((connection.clone(), session.clone()));
    }
    listen(&connection, &session, events)
}

/// Calls a portal method that answers through a Request object, and waits for that answer (the user may be choosing
/// keys). The Response is subscribed to before the call, so it can't be missed.
fn request<B>(connection: &Connection, token: &str, method: &str, body: &B) -> zbus::Result<HashMap<String, OwnedValue>>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let sender =
        connection.unique_name().map(|name| name.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let path = format!("{PORTAL_PATH}/request/{sender}/{token}");
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")?
        .member("Response")?
        .path(path)?
        .build();
    let mut answers = MessageIterator::for_match_rule(rule, connection, Some(1))?;
    connection.call_method(Some(PORTAL), PORTAL_PATH, Some(GLOBAL_SHORTCUTS), method, body)?;
    let answer = answers.next().ok_or_else(|| zbus::Error::Failure(format!("no answer to {method}")))??;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize()?;
    match code {
        0 => Ok(results),
        1 => Err(zbus::Error::Failure(format!("{method} was cancelled"))),
        _ => Err(zbus::Error::Failure(format!("{method} failed"))),
    }
}

fn take<T>(results: &HashMap<String, OwnedValue>, key: &str) -> zbus::Result<T>
where
    T: TryFrom<OwnedValue>,
{
    let value = results.get(key).ok_or_else(|| zbus::Error::Failure(format!("no {key} in the answer")))?;
    T::try_from(value.try_clone()?).map_err(|_| zbus::Error::Failure(format!("{key} has the wrong type")))
}

/// Turns the session's `Activated` (key down) and `Deactivated` (key up) into requests, for as long as it's open.
fn listen(connection: &Connection, session: &OwnedObjectPath, events: &Sender<Event>) -> zbus::Result<()> {
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(GLOBAL_SHORTCUTS)?.build();
    for message in MessageIterator::for_match_rule(rule, connection, None)?.flatten() {
        let header = message.header();
        let down = match header.member().map(zbus::names::MemberName::as_str) {
            Some("Activated") => true,
            Some("Deactivated") => false,
            _ => continue,
        };
        let Ok((from, id, _, _)) =
            message.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if from != *session {
            continue;
        }
        if let Some(request) = Shortcut::from_id(&id).and_then(|shortcut| shortcut.request(Some(down))) {
            if events.send(Event::Remote(request)).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}
