#!/usr/bin/env bash
# In-app update check: a test copy of the service (no mixer, own socket) updates from a fake release served on
# localhost, asked over the window protocol. The fake release's install.sh only leaves a marker file, so nothing is
# really installed. Usage: tests/live_update.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
work=$(mktemp -d)
port=$(python3 -I -c "import socket; s = socket.socket(); s.bind(('127.0.0.1', 0)); print(s.getsockname()[1])")
base="http://127.0.0.1:$port"

# a release tarball whose install.sh leaves `$1` behind
release() {
  mkdir -p "$work/build/cranked-audio-v99.0.0-x86_64"
  printf '#!/bin/sh\ntouch "%s"\n' "$1" > "$work/build/cranked-audio-v99.0.0-x86_64/install.sh"
  tar -czf "$work/release/cranked-audio-v99.0.0-x86_64.tar.gz" -C "$work/build" cranked-audio-v99.0.0-x86_64
}
mkdir -p "$work/release"
release "$work/installed"
(cd "$work/release" && sha256sum cranked-audio-v99.0.0-x86_64.tar.gz > cranked-audio-v99.0.0-x86_64.tar.gz.sha256)
cat > "$work/release/latest.json" <<JSON
{"tag_name": "v99.0.0", "assets": [
  {"name": "cranked-audio-v99.0.0-windows-x86_64-experimental.zip", "browser_download_url": "$base/missing.zip"},
  {"name": "cranked-audio-v99.0.0-x86_64.tar.gz", "browser_download_url": "$base/cranked-audio-v99.0.0-x86_64.tar.gz"},
  {"name": "cranked-audio-v99.0.0-x86_64.tar.gz.sha256",
   "browser_download_url": "$base/cranked-audio-v99.0.0-x86_64.tar.gz.sha256"}
]}
JSON
python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$work/release" > /dev/null 2>&1 &
server=$!

printf 'popups = false\n[[channel]]\nname = "Test"\napps = ["cranked-selftest-none"]\n' > "$work/config.toml"
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state" CRANKED_DBUS_NAME="io.github.zach9113.CrankedAudio.Test$$"
export CRANKED_UPDATE_API="$base/latest.json"
"$bin" --config "$work/config.toml" --no-mixer > "$work/log" 2>&1 &
daemon=$!
sleep 1.5

# sends window lines and prints the update status that answers each one (the service sends no interim ones)
window() {
  python3 -I - "$CRANKED_SOCKET" "$@" <<'PY'
import json, socket, sys, time
connection = socket.socket(socket.AF_UNIX)
connection.connect(sys.argv[1])
connection.settimeout(20)
replies = connection.makefile("r")
for line in sys.argv[2:]:
    connection.sendall(line.encode() + b"\n")
    deadline = time.time() + 20
    while time.time() < deadline:
        message = json.loads(replies.readline())
        if isinstance(message, dict) and "UpdateStatus" in message:
            print(message["UpdateStatus"]["status"])
            break
    else:
        print("no answer")
PY
}
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }

check "finds the newer release" "$(window check-update)" "Update available: v99.0.0"
check "installs it" "$(window apply-update)" "Update installed! Restarting..."
check "by running the release's install.sh" "$([ -e "$work/installed" ] && echo yes || echo no)" "yes"

# the same release, tampered with after its checksum was published
release "$work/tampered"
check "a download that doesn't match the checksum isn't installed" "$(window apply-update)" \
  "Install failed: The download doesn't match the release's checksum; nothing was installed"
check "and its installer never runs" "$([ -e "$work/tampered" ] && echo yes || echo no)" "no"

kill $daemon $server 2>/dev/null; wait 2>/dev/null; rm -rf "$work"
echo "failures: $fails"; exit $fails
