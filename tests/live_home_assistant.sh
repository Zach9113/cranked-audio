#!/usr/bin/env bash
# End-to-end Home Assistant check against tests/fake_home_assistant.py (no real Home Assistant, no mixer, no audio).
# Usage: tests/live_home_assistant.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
here=$(dirname "$0")
work=$(mktemp -d)
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state"
port=$((20000 + RANDOM % 20000))
echo "fake-token-$RANDOM" > "$work/ha-token"; chmod 600 "$work/ha-token"
cat > "$work/config.toml" <<TOML
popups = false
[home_assistant]
url = "http://127.0.0.1:$port"
[[channel]]
name = "Lamp"
light = "light.test_lamp"
press = { ha_toggle = "switch.test_plug" }
hold = "none"
TOML
touch "$work/calls"
python3 "$here/fake_home_assistant.py" "$port" "$(cat "$work/ha-token")" "$work/calls" &
fake=$!
sleep 0.5
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }
external() { curl -s -X POST "http://127.0.0.1:$port/test/set" -d "$1" > /dev/null; }

mkfifo "$work/in"
"$bin" --config "$work/config.toml" --no-mixer --stdin -v < "$work/in" 2> "$work/log" &
daemon=$!
exec 3> "$work/in"
sleep 1
echo "knob 0 40" >&3; sleep 0.4
check "first turn sets the lamp" "$(tail -1 "$work/calls")" "light/turn_on light.test_lamp 40"
external '{"entity_id": "light.test_lamp", "state": "on", "brightness": 80}'; sleep 0.3
echo "knob 0 70" >&3; sleep 0.4
check "lamp changed elsewhere: knob catches up instead of jumping" "$(tail -1 "$work/calls")" "light/turn_on light.test_lamp 90"
before=$(wc -l < "$work/calls")
for p in $(seq 69 -1 50); do echo "knob 0 $p" >&3; sleep 0.01; done; sleep 0.5
sent=$(( $(wc -l < "$work/calls") - before ))
check "fast turn is throttled (20 moves -> at most 5 calls)" "$([ "$sent" -le 5 ] && [ "$sent" -ge 1 ] && echo yes || echo "no, $sent")" "yes"
check "and ends on the newest value" "$(tail -1 "$work/calls" | awk '{print $1, $2}')" "light/turn_on light.test_lamp"
echo "knob 0 0" >&3; sleep 0.4
check "knob at 0 turns the lamp off" "$(tail -1 "$work/calls")" "light/turn_off light.test_lamp"
echo "down 0" >&3; sleep 0.05; echo "up 0" >&3; sleep 0.4
check "button toggles the plug" "$(tail -1 "$work/calls")" "switch/toggle switch.test_plug"
check "live updates connected" "$(grep -c 'live updates connected' "$work/log")" "1"
sleep 2
t0=$(awk '{print $14+$15}' /proc/$daemon/stat); sleep 3; t1=$(awk '{print $14+$15}' /proc/$daemon/stat)
echo "idle: $((t1 - t0)) cpu ticks in 3 s, rss $(awk '/VmRSS/{print $2" "$3}' /proc/$daemon/status)"
echo "--- calls:"; cat "$work/calls"
echo "--- daemon log:"; grep -v "^stream " "$work/log"
exec 3>&-; kill $daemon $fake 2>/dev/null; wait 2>/dev/null; rm -rf "$work"
echo "failures: $fails"; exit $fails
