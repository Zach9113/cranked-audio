//! The window's connection (see the `cranked-ipc` crate). Also makes sure only one service runs.

use std::io::{BufRead as _, BufReader, Write as _};
use std::sync::mpsc::Sender;
use std::time::Duration;

use cranked_ipc::{ServerMessage, UnixListener, UnixStream};

use crate::Event;

/// Binds the socket, or returns None when another copy is already running.
pub fn listen() -> Option<UnixListener> {
    let path = cranked_ipc::socket_path();
    if UnixStream::connect(&path).is_ok() {
        return None;
    }
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("window socket: couldn't remove stale {} ({e})", path.display());
        }
    }
    match UnixListener::bind(&path) {
        Ok(listener) => Some(listener),
        Err(e) => {
            eprintln!("window socket: can't listen on {} ({e})", path.display());
            None
        }
    }
}

pub fn run(listener: &UnixListener, events: &Sender<Event>) {
    let mut next_id = 0u64;
    for client in listener.incoming().map_while(Result::ok) {
        let Ok(writer) = client.try_clone() else { continue };
        let _ = writer.set_write_timeout(Some(Duration::from_millis(200)));
        next_id += 1;
        let id = next_id;
        if events.send(Event::ClientConnected(id, writer)).is_err() {
            return;
        }
        let events = events.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(client).lines().map_while(Result::ok) {
                if events.send(Event::ClientLine(line)).is_err() {
                    return;
                }
            }
            let _ = events.send(Event::ClientDisconnected(id));
        });
    }
}

/// Connected windows; each gets every new snapshot as one JSON line.
#[derive(Default)]
pub struct Clients {
    streams: Vec<(u64, UnixStream)>,
    last: Option<String>,
}

impl Clients {
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    pub fn add(&mut self, id: u64, mut stream: UnixStream, snapshot: &cranked_ipc::Snapshot) {
        let line = serde_json::to_string(&ServerMessage::Snapshot(snapshot.clone())).unwrap_or_default();
        if writeln!(stream, "{line}").is_ok() {
            self.streams.push((id, stream));
            self.last = Some(line);
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.streams.retain(|(cid, _)| *cid != id);
    }

    /// Sends the snapshot if it changed; drops windows that went away.
    pub fn update(&mut self, snapshot: &cranked_ipc::Snapshot) {
        let line = serde_json::to_string(&ServerMessage::Snapshot(snapshot.clone())).unwrap_or_default();
        if self.last.as_deref() == Some(line.as_str()) {
            return;
        }
        self.streams.retain_mut(|(_, stream)| writeln!(stream, "{line}").is_ok());
        self.last = Some(line);
    }

    pub fn send(&mut self, message: &ServerMessage) {
        let line = serde_json::to_string(message).unwrap_or_default();
        self.streams.retain_mut(|(_, stream)| writeln!(stream, "{line}").is_ok());
    }
}
