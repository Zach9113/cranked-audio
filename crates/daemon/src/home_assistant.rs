//! Home Assistant (SPEC.md section 7): service calls out, live entity state in.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use ureq::Agent;

use crate::Event;
use cranked_protocol::convert::round_u8;

/// Light changes go out at most this often per light; the newest value wins.
const LIGHT_EVERY: Duration = Duration::from_millis(150);
const POLL_EVERY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Settings {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Light brightness in percent; 0 turns it off.
    Brightness { entity: String, percent: u8 },
    /// `toggle` or `turn_off` on any entity; scenes and scripts are turned on instead.
    Call { service: &'static str, entity: String },
    /// Answer with `Event::HomeAssistantEntities`: everything a knob or button can use.
    ListEntities,
}

/// Domains a button can toggle or a knob can dim.
const USABLE: &[&str] = &["light", "switch", "fan", "input_boolean", "scene", "script"];

fn list_entities(agent: &Agent, settings: &Settings) -> Result<Vec<cranked_ipc::Entity>, String> {
    let mut response = agent
        .get(format!("{}/api/states", settings.url))
        .header("Authorization", format!("Bearer {}", settings.token))
        .call()
        .map_err(|e| e.to_string())?;
    let text = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let states: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut entities: Vec<cranked_ipc::Entity> = states
        .iter()
        .filter_map(parse_state)
        .filter(|s| USABLE.contains(&s.entity.split('.').next().unwrap_or_default()))
        .map(|s| cranked_ipc::Entity { id: s.entity, name: s.name })
        .collect();
    entities.sort_by_key(|a| a.name.to_lowercase());
    Ok(entities)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityState {
    pub entity: String,
    pub name: String,
    /// `on`, `off`, `unavailable`, ...
    pub state: String,
    /// Lights: 0..100 (0 when off).
    pub brightness: Option<u8>,
}

impl EntityState {
    pub fn is_on(&self) -> bool {
        self.state == "on"
    }
}

pub fn parse_state(value: &Value) -> Option<EntityState> {
    let entity = value.get("entity_id")?.as_str()?.to_string();
    let state = value.get("state")?.as_str()?.to_string();
    let attributes = value.get("attributes");
    let name = attributes.and_then(|a| a.get("friendly_name")).and_then(Value::as_str).unwrap_or(&entity).to_string();
    let brightness = entity.starts_with("light.").then(|| {
        if state != "on" {
            return 0;
        }
        let raw = attributes.and_then(|a| a.get("brightness")).and_then(Value::as_f64).unwrap_or(255.0);
        round_u8(raw * 100.0 / 255.0).clamp(1, 100)
    });
    Some(EntityState { entity, name, state, brightness })
}

/// Which service actually runs for a button: (domain, service).
pub fn service_for(entity: &str, service: &str) -> (String, String) {
    let domain = entity.split('.').next().unwrap_or_default();
    match domain {
        "scene" | "script" => (domain.to_string(), "turn_on".to_string()),
        "switch" | "light" | "fan" | "input_boolean" | "media_player" | "cover" => {
            (domain.to_string(), service.to_string())
        }
        _ => ("homeassistant".to_string(), service.to_string()),
    }
}

fn agent(timeout: Option<Duration>) -> Agent {
    Agent::config_builder().timeout_global(timeout).http_status_as_error(true).build().into()
}

fn post(agent: &Agent, settings: &Settings, path: &str, body: &Value) -> Result<(), String> {
    agent
        .post(format!("{}{path}", settings.url))
        .header("Authorization", format!("Bearer {}", settings.token))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn get_state(agent: &Agent, settings: &Settings, entity: &str) -> Option<EntityState> {
    let mut response = agent
        .get(format!("{}/api/states/{entity}", settings.url))
        .header("Authorization", format!("Bearer {}", settings.token))
        .call()
        .ok()?;
    let text = response.body_mut().read_to_string().ok()?;
    parse_state(&serde_json::from_str(&text).ok()?)
}

fn call(agent: &Agent, settings: &Settings, command: &Command) -> Result<(), String> {
    match command {
        Command::Brightness { entity, percent: 0 } => {
            post(agent, settings, "/api/services/light/turn_off", &json!({ "entity_id": entity }))
        }
        Command::Brightness { entity, percent } => post(
            agent,
            settings,
            "/api/services/light/turn_on",
            &json!({ "entity_id": entity, "brightness_pct": percent }),
        ),
        Command::Call { service, entity } => {
            let (domain, service) = service_for(entity, service);
            post(agent, settings, &format!("/api/services/{domain}/{service}"), &json!({ "entity_id": entity }))
        }
        Command::ListEntities => Ok(()),
    }
}

/// Sends service calls, holding back light changes so each light gets at most one per `LIGHT_EVERY`.
pub fn run_calls(settings: &Settings, commands: &Receiver<Command>, events: &Sender<Event>) {
    let agent = agent(Some(Duration::from_secs(10)));
    let mut waiting: HashMap<String, u8> = HashMap::new();
    let mut last_sent: HashMap<String, Instant> = HashMap::new();
    loop {
        let wait = if waiting.is_empty() { Duration::from_secs(3600) } else { Duration::from_millis(20) };
        match commands.recv_timeout(wait) {
            Ok(Command::Brightness { entity, percent }) => {
                waiting.insert(entity, percent);
            }
            Ok(Command::ListEntities) => match list_entities(&agent, settings) {
                Ok(entities) => {
                    let _ = events.send(Event::HomeAssistantEntities(entities));
                }
                Err(e) => eprintln!("home assistant: can't list entities ({e})"),
            },
            Ok(command) => {
                if let Err(e) = call(&agent, settings, &command) {
                    eprintln!("home assistant: {e}");
                    let _ = events.send(Event::HomeAssistantFailed);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due: Vec<String> = waiting
            .keys()
            .filter(|e| last_sent.get(*e).is_none_or(|t| now.duration_since(*t) >= LIGHT_EVERY))
            .cloned()
            .collect();
        for entity in due {
            let Some(percent) = waiting.remove(&entity) else { continue };
            last_sent.insert(entity.clone(), now);
            if let Err(e) = call(&agent, settings, &Command::Brightness { entity, percent }) {
                eprintln!("home assistant: {e}");
                let _ = events.send(Event::HomeAssistantFailed);
            }
        }
    }
}

/// Home Assistant's compact entity updates (`subscribe_entities`): `a` = full states, `c` = changes with `+`
/// (new values) and `-` (removed attributes), `r` = removed entities. Keeps `known` current and returns the
/// entities that changed, as ordinary states.
pub fn apply_update(
    known: &mut HashMap<String, (String, serde_json::Map<String, Value>)>,
    event: &Value,
) -> Vec<EntityState> {
    let mut changed = Vec::new();
    if let Some(added) = event.get("a").and_then(Value::as_object) {
        for (id, full) in added {
            let state = full.get("s").and_then(Value::as_str).unwrap_or_default().to_string();
            let attributes = full.get("a").and_then(Value::as_object).cloned().unwrap_or_default();
            known.insert(id.clone(), (state, attributes));
            changed.push(id.clone());
        }
    }
    if let Some(changes) = event.get("c").and_then(Value::as_object) {
        for (id, change) in changes {
            let entry = known.entry(id.clone()).or_default();
            if let Some(plus) = change.get("+") {
                if let Some(state) = plus.get("s").and_then(Value::as_str) {
                    entry.0 = state.to_string();
                }
                if let Some(attributes) = plus.get("a").and_then(Value::as_object) {
                    entry.1.extend(attributes.clone());
                }
            }
            if let Some(gone) = change.pointer("/-/a").and_then(Value::as_array) {
                for name in gone.iter().filter_map(Value::as_str) {
                    entry.1.remove(name);
                }
            }
            changed.push(id.clone());
        }
    }
    changed.sort();
    changed.dedup();
    changed
        .iter()
        .filter_map(|id| {
            let (state, attributes) = known.get(id)?;
            parse_state(&json!({ "entity_id": id, "state": state, "attributes": attributes }))
        })
        .collect()
}

/// `http://host:8123` -> `ws://host:8123/api/websocket` (https -> wss).
pub fn websocket_url(base: &str) -> String {
    let rest = base.strip_prefix("https://").map(|r| format!("wss://{r}"));
    let url = rest.unwrap_or_else(|| format!("ws://{}", base.strip_prefix("http://").unwrap_or(base)));
    format!("{}/api/websocket", url.trim_end_matches('/'))
}

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// A quiet connection gets a ping after this long; no answer within the same time means it's dead.
const PING_AFTER: Duration = Duration::from_secs(30);

fn read_json(socket: &mut Socket) -> Result<Option<Value>, String> {
    match socket.read() {
        Ok(tungstenite::Message::Text(text)) => Ok(serde_json::from_str(text.as_str()).ok()),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(e))
            if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
        {
            Err("timeout".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn send_json(socket: &mut Socket, value: &Value) -> Result<(), String> {
    socket.send(tungstenite::Message::text(value.to_string())).map_err(|e| e.to_string())
}

/// Connects, signs in and subscribes to just the watched entities.
fn subscribe(settings: &Settings, watched: &[String]) -> Result<Socket, String> {
    let (mut socket, _) = tungstenite::connect(websocket_url(&settings.url)).map_err(|e| e.to_string())?;
    let tcp = match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(tcp) => tcp,
        tungstenite::stream::MaybeTlsStream::Rustls(tls) => tls.get_ref(),
        _ => return Err("unsupported connection".into()),
    };
    tcp.set_read_timeout(Some(PING_AFTER)).map_err(|e| e.to_string())?;
    let _ = read_json(&mut socket)?; // auth_required
    send_json(&mut socket, &json!({ "type": "auth", "access_token": settings.token }))?;
    match read_json(&mut socket)? {
        Some(reply) if reply["type"] == "auth_ok" => {}
        _ => return Err("Home Assistant didn't accept the token".into()),
    }
    send_json(&mut socket, &json!({ "id": 1, "type": "subscribe_entities", "entity_ids": watched }))?;
    Ok(socket)
}

/// Keeps `watched` entities' state current over Home Assistant's WebSocket (only those entities, with pings to notice
/// a dead connection); polls while it can't connect. Stops once `generation` moves past `mine`.
pub fn run_states(settings: &Settings, watched: &[String], events: &Sender<Event>, generation: &AtomicU64, mine: u64) {
    if watched.is_empty() {
        return;
    }
    let current = || generation.load(Ordering::Relaxed) == mine;
    let poller = agent(Some(Duration::from_secs(10)));
    let mut reported = false;
    while current() {
        let mut socket = match subscribe(settings, watched) {
            Ok(socket) => socket,
            Err(e) => {
                if !reported {
                    eprintln!(
                        "home assistant: live updates unavailable ({e}); checking every {} s",
                        POLL_EVERY.as_secs()
                    );
                    reported = true;
                }
                for entity in watched {
                    if let Some(state) = get_state(&poller, settings, entity) {
                        if events.send(Event::HomeAssistantState(state)).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(POLL_EVERY);
                continue;
            }
        };
        eprintln!("home assistant: live updates connected");
        reported = false;
        let mut known = HashMap::new();
        let (mut next_id, mut waiting_for_pong) = (2_u64, false);
        let reason = loop {
            if !current() {
                let _ = socket.close(None);
                return;
            }
            match read_json(&mut socket) {
                Ok(Some(message)) => {
                    waiting_for_pong = false;
                    if message["type"] == "event" && message["id"] == 1 {
                        for state in apply_update(&mut known, &message["event"]) {
                            if events.send(Event::HomeAssistantState(state)).is_err() {
                                return;
                            }
                        }
                    } else if message["type"] == "result" && message["success"] == false {
                        break format!("subscription refused: {}", message["error"]["message"]);
                    }
                }
                Ok(None) => {}
                Err(e) if e == "timeout" && !waiting_for_pong => {
                    waiting_for_pong = true;
                    if let Err(e) = send_json(&mut socket, &json!({ "id": next_id, "type": "ping" })) {
                        break e;
                    }
                    next_id += 1;
                }
                Err(e) if e == "timeout" => break "no answer to a ping".into(),
                Err(e) => break e,
            }
        };
        eprintln!("home assistant: live updates dropped ({reason}); reconnecting");
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_light_and_switch_states() {
        let lamp = parse_state(&json!({"entity_id": "light.desk_lamp", "state": "on",
            "attributes": {"brightness": 89, "friendly_name": "Desk Lamp"}}))
        .unwrap();
        assert_eq!((lamp.name.as_str(), lamp.brightness), ("Desk Lamp", Some(35)));
        let off = parse_state(&json!({"entity_id": "light.x", "state": "off", "attributes": {}})).unwrap();
        assert_eq!(off.brightness, Some(0));
        let dim =
            parse_state(&json!({"entity_id": "light.x", "state": "on", "attributes": {"brightness": 1}})).unwrap();
        assert_eq!(dim.brightness, Some(1), "on is never shown as 0");
        let switch = parse_state(&json!({"entity_id": "switch.coffee_maker", "state": "on"})).unwrap();
        assert!(switch.is_on() && switch.brightness.is_none() && switch.name == "switch.coffee_maker");
    }

    #[test]
    fn buttons_pick_the_right_service() {
        assert_eq!(service_for("switch.coffee_maker", "toggle"), ("switch".into(), "toggle".into()));
        assert_eq!(service_for("scene.movie", "toggle"), ("scene".into(), "turn_on".into()));
        assert_eq!(service_for("script.bedtime", "turn_off"), ("script".into(), "turn_on".into()));
        assert_eq!(service_for("group.lights", "toggle"), ("homeassistant".into(), "toggle".into()));
    }

    #[test]
    fn compact_updates_keep_state_and_attributes() {
        let mut known = HashMap::new();
        let added = apply_update(
            &mut known,
            &json!({"a": {"light.desk_lamp": {"s": "on", "a": {"brightness": 89, "friendly_name": "Desk Lamp"}, "c": "x", "lc": 1.0}}}),
        );
        assert_eq!((added[0].name.as_str(), added[0].brightness), ("Desk Lamp", Some(35)));
        let dimmed =
            apply_update(&mut known, &json!({"c": {"light.desk_lamp": {"+": {"a": {"brightness": 255}, "c": "y"}}}}));
        assert_eq!(dimmed[0].brightness, Some(100), "attributes change without a state change");
        let off = apply_update(
            &mut known,
            &json!({"c": {"light.desk_lamp": {"+": {"s": "off"}, "-": {"a": ["brightness"]}}}}),
        );
        assert_eq!((off[0].state.as_str(), off[0].brightness, off[0].name.as_str()), ("off", Some(0), "Desk Lamp"));
    }

    #[test]
    fn websocket_addresses() {
        assert_eq!(websocket_url("http://192.0.2.1:8123"), "ws://192.0.2.1:8123/api/websocket");
        assert_eq!(
            websocket_url("https://home-assistant.example.org/"),
            "wss://home-assistant.example.org/api/websocket"
        );
    }
}
