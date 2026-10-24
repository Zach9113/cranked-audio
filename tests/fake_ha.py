#!/usr/bin/env python3
"""A tiny stand-in for Home Assistant's REST API and WebSocket API (subscribe_entities), for tests/live_ha.sh.

Usage: fake_ha.py PORT TOKEN LOG. Every service call is appended to LOG as `domain/service entity [brightness]`.
POST /test/set {"entity_id", "state", "brightness"} changes a state as if someone used another app."""
import base64, hashlib, json, queue, struct, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port, token, log_path = int(sys.argv[1]), sys.argv[2], sys.argv[3]
states = {
    "light.test_lamp": {"entity_id": "light.test_lamp", "state": "off", "attributes": {"friendly_name": "Test Lamp"}},
    "switch.test_plug": {"entity_id": "switch.test_plug", "state": "off", "attributes": {"friendly_name": "Test Plug"}},
}
listeners, lock = [], threading.Lock()

def compact(entity_id):
    s = states[entity_id]
    return {"s": s["state"], "a": dict(s["attributes"]), "c": "ctx", "lc": 0}

def publish(entity_id):
    # like Home Assistant: a change carries the new state and all attributes ("+"), and lists dropped ones ("-")
    s = states[entity_id]
    change = {"+": {"s": s["state"], "a": dict(s["attributes"])}}
    if "brightness" not in s["attributes"]:
        change["-"] = {"a": ["brightness"]}
    with lock:
        for q, subscribed in listeners:
            if entity_id in subscribed:
                q.put({"c": {entity_id: change}})

def ws_recv(sock):
    head = sock.recv(2)
    if len(head) < 2:
        raise OSError("closed")
    length = head[1] & 0x7F
    if length == 126:
        length = struct.unpack(">H", sock.recv(2))[0]
    elif length == 127:
        length = struct.unpack(">Q", sock.recv(8))[0]
    mask = sock.recv(4)
    data = b""
    while len(data) < length:
        data += sock.recv(length - len(data))
    if head[0] & 0x0F == 8:
        raise OSError("closed")
    return json.loads(bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

def ws_send(sock, message):
    data = json.dumps(message).encode()
    head = bytes([0x81]) + (bytes([len(data)]) if len(data) < 126 else bytes([126]) + struct.pack(">H", len(data)))
    sock.sendall(head + data)

def set_light(entity, on, pct=None):
    attributes = states[entity]["attributes"]
    states[entity]["state"] = "on" if on else "off"
    if on and pct is not None:
        attributes["brightness"] = round(pct * 255 / 100)
    elif not on:
        attributes.pop("brightness", None)

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"  # WebSockets need 1.1

    def log_message(self, *args):
        pass

    def authorized(self):
        if self.headers.get("Authorization") != f"Bearer {token}":
            self.send_response(401); self.send_header("Content-Length", "0"); self.end_headers(); return False
        return True

    def reply(self, body):
        data = json.dumps(body).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)

    def do_GET(self):
        if self.path == "/api/websocket":
            return self.websocket()  # signs in inside the connection, like Home Assistant
        if not self.authorized():
            return
        if self.path.startswith("/api/states/"):
            return self.reply(states[self.path.rsplit("/", 1)[1]])

    def websocket(self):
        accept = base64.b64encode(hashlib.sha1((self.headers["Sec-WebSocket-Key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
        self.send_response(101); self.send_header("Upgrade", "websocket"); self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept.decode()); self.end_headers(); self.wfile.flush()
        sock = self.connection
        ws_send(sock, {"type": "auth_required"})
        if ws_recv(sock).get("access_token") != token:
            return ws_send(sock, {"type": "auth_invalid"})
        ws_send(sock, {"type": "auth_ok"})
        q, subscribed = queue.Queue(), set()
        entry = (q, subscribed)
        def writer():
            while True:
                event = q.get()
                if event is None:
                    return
                try:
                    ws_send(sock, {"id": 1, "type": "event", "event": event})
                except OSError:
                    return
        threading.Thread(target=writer, daemon=True).start()
        try:
            while True:
                message = ws_recv(sock)
                if message.get("type") == "subscribe_entities":
                    subscribed.update(message["entity_ids"])
                    ws_send(sock, {"id": message["id"], "type": "result", "success": True, "result": None})
                    with lock:
                        listeners.append(entry)
                    q.put({"a": {e: compact(e) for e in subscribed if e in states}})
                elif message.get("type") == "ping":
                    ws_send(sock, {"id": message["id"], "type": "pong"})
        except OSError:
            pass
        finally:
            q.put(None)
            with lock:
                if entry in listeners:
                    listeners.remove(entry)

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
