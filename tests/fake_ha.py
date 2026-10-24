#!/usr/bin/env python3
"""A tiny stand-in for Home Assistant's REST API and event stream, for tests/live_ha.sh.

Usage: fake_ha.py PORT TOKEN LOG. Every service call is appended to LOG as `domain/service entity [brightness]`.
POST /test/set {"entity_id", "state", "brightness"} changes a state as if someone used another app."""
import json, queue, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port, token, log_path = int(sys.argv[1]), sys.argv[2], sys.argv[3]
states = {
    "light.test_lamp": {"entity_id": "light.test_lamp", "state": "off", "attributes": {"friendly_name": "Test Lamp"}},
    "switch.test_plug": {"entity_id": "switch.test_plug", "state": "off", "attributes": {"friendly_name": "Test Plug"}},
}
listeners, lock = [], threading.Lock()

def publish(entity_id):
    line = "data: " + json.dumps({"event_type": "state_changed", "data": {"entity_id": entity_id, "new_state": states[entity_id]}})
    noise = "data: " + json.dumps({"event_type": "state_changed", "data": {"entity_id": "sensor.other", "new_state": {"entity_id": "sensor.other", "state": "1"}}})
    with lock:
        for q in listeners:
            q.put(noise); q.put(line)

def set_light(entity, on, pct=None):
    attributes = states[entity]["attributes"]
    states[entity]["state"] = "on" if on else "off"
    if on and pct is not None:
        attributes["brightness"] = round(pct * 255 / 100)
    elif not on:
        attributes.pop("brightness", None)

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def authorized(self):
        if self.headers.get("Authorization") != f"Bearer {token}":
            self.send_response(401); self.end_headers(); return False
        return True

    def reply(self, body):
        data = json.dumps(body).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)

    def do_GET(self):
        if not self.authorized():
            return
        if self.path.startswith("/api/states/"):
            return self.reply(states[self.path.rsplit("/", 1)[1]])
        if self.path.startswith("/api/stream"):
            self.send_response(200); self.send_header("Content-Type", "text/event-stream"); self.end_headers()
            q = queue.Queue()
            with lock:
                listeners.append(q)
            try:
                self.wfile.write(b"data: ping\n\n"); self.wfile.flush()
                while True:
                    self.wfile.write((q.get() + "\n\n").encode()); self.wfile.flush()
            except OSError:
                with lock:
                    listeners.remove(q)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        if self.path == "/test/set":
            entity = body["entity_id"]
            set_light(entity, body["state"] == "on", body.get("brightness")) if entity.startswith("light.") else states[entity].update(state=body["state"])
            publish(entity); return self.reply({})
        if not self.authorized():
            return
        _, _, domain, service = self.path.split("/")[1:5] if self.path.count("/") >= 4 else ("", "", "", "")
        entity = body.get("entity_id")
        with open(log_path, "a") as log:
            log.write(f"{domain}/{service} {entity}" + (f" {body['brightness_pct']}" if "brightness_pct" in body else "") + "\n")
        if domain == "light":
            set_light(entity, service == "turn_on", body.get("brightness_pct"))
        elif service == "toggle":
            states[entity]["state"] = "off" if states[entity]["state"] == "on" else "on"
        publish(entity)
        self.reply([])

ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
