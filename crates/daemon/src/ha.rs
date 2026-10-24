//! Home Assistant (SPEC.md section 7): service calls out, live entity state in.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use ureq::Agent;

use crate::Event;

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
    /// Answer with `Event::HaEntities`: everything a knob or button can use.
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
    entities.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
    let name = attributes
        .and_then(|a| a.get("friendly_name"))
        .and_then(Value::as_str)
        .unwrap_or(&entity)
        .to_string();
    let brightness = entity.starts_with("light.").then(|| {
        if state != "on" {
            return 0;
        }
        let raw = attributes.and_then(|a| a.get("brightness")).and_then(Value::as_f64).unwrap_or(255.0);
        ((raw * 100.0 / 255.0).round() as u8).clamp(1, 100)
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

fn post(agent: &Agent, settings: &Settings, path: &str, body: Value) -> Result<(), String> {
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
            post(agent, settings, "/api/services/light/turn_off", json!({ "entity_id": entity }))
        }
        Command::Brightness { entity, percent } => post(
            agent,
            settings,
            "/api/services/light/turn_on",
            json!({ "entity_id": entity, "brightness_pct": percent }),
        ),
        Command::Call { service, entity } => {
            let (domain, service) = service_for(entity, service);
            post(agent, settings, &format!("/api/services/{domain}/{service}"), json!({ "entity_id": entity }))
        }
        Command::ListEntities => Ok(()),
    }
}

/// Sends service calls, holding back light changes so each light gets at most one per `LIGHT_EVERY`.
pub fn run_calls(settings: Settings, commands: Receiver<Command>, events: Sender<Event>) {
    let agent = agent(Some(Duration::from_secs(10)));
    let mut waiting: HashMap<String, u8> = HashMap::new();
    let mut last_sent: HashMap<String, Instant> = HashMap::new();
    loop {
        let wait = if waiting.is_empty() { Duration::from_secs(3600) } else { Duration::from_millis(20) };
        match commands.recv_timeout(wait) {
            Ok(Command::Brightness { entity, percent }) => {
                waiting.insert(entity, percent);
            }
            Ok(Command::ListEntities) => match list_entities(&agent, &settings) {
                Ok(entities) => {
                    let _ = events.send(Event::HaEntities(entities));
                }
                Err(e) => eprintln!("home assistant: can't list entities ({e})"),
            },
            Ok(command) => {
                if let Err(e) = call(&agent, &settings, &command) {
                    eprintln!("home assistant: {e}");
                    let _ = events.send(Event::HaFailed);
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
            if let Err(e) = call(&agent, &settings, &Command::Brightness { entity, percent }) {
                eprintln!("home assistant: {e}");
                let _ = events.send(Event::HaFailed);
            }
        }
    }
}

/// Keeps `watched` entities' state current: the live event stream, with polling while it's down.
/// Stops once `generation` moves past `mine` (new settings started a replacement).
pub fn run_states(settings: Settings, watched: Vec<String>, events: Sender<Event>, generation: Arc<AtomicU64>, mine: u64) {
    let current = || generation.load(Ordering::Relaxed) == mine;
    if watched.is_empty() {
        return;
    }
    let poller = agent(Some(Duration::from_secs(10)));
    // no overall timeout for the stream; HA sends a few events a second on a typical install
    let streamer: Agent =
        Agent::config_builder().timeout_recv_body(Some(Duration::from_secs(120))).http_status_as_error(true).build().into();
    let markers: Vec<String> = watched.iter().map(|e| format!("\"{e}\"")).collect();
    let mut last_poll: Option<Instant> = None;
    while current() {
        if last_poll.is_none_or(|t| t.elapsed() >= POLL_EVERY) {
            last_poll = Some(Instant::now());
            for entity in &watched {
                if let Some(state) = get_state(&poller, &settings, entity) {
                    if events.send(Event::HaState(state)).is_err() {
                        return;
                    }
                }
            }
        }
        let response = streamer
            .get(format!("{}/api/stream?restrict=state_changed", settings.url))
            .header("Authorization", format!("Bearer {}", settings.token))
            .call();
        let response = match response {
            Ok(response) => response,
            Err(e) => {
                eprintln!("home assistant: live updates unavailable ({e}); polling");
                std::thread::sleep(POLL_EVERY);
                continue;
            }
        };
        eprintln!("home assistant: live updates connected");
        let reader = BufReader::new(response.into_body().into_reader());
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if !current() {
                return;
            }
            let Some(data) = line.strip_prefix("data: ") else { continue };
            // most events are about other entities; skip them without parsing
            if !markers.iter().any(|m| data.contains(m.as_str())) {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            if let Some(state) = event.pointer("/data/new_state").and_then(parse_state) {
                if events.send(Event::HaState(state)).is_err() {
                    return;
                }
            }
        }
        eprintln!("home assistant: live updates dropped; reconnecting");
        last_poll = None;
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
        let dim = parse_state(&json!({"entity_id": "light.x", "state": "on", "attributes": {"brightness": 1}})).unwrap();
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
}
