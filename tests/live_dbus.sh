#!/usr/bin/env bash
# D-Bus API check: a test copy of the service (no mixer, test settings, own socket) driven over the session bus.
# Usage: tests/live_dbus.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
work=$(mktemp -d)
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state" CRANKED_DBUS_NAME="io.github.zach9113.CrankedAudio.Test$$"
cat > "$work/config.toml" <<'TOML'
popups = false
[[profile]]
name = "One"
[[profile.channel]]
name = "Test"
apps = ["cranked-selftest-none"]
hold = "mute"
[[profile]]
name = "Two"
[[profile.channel]]
name = "Other"
TOML
"$bin" --config "$work/config.toml" --no-mixer > "$work/log" 2>&1 &
daemon=$!
sleep 1.5
call() { busctl --user --json=short call -- "$CRANKED_DBUS_NAME" /io/github/zach9113/CrankedAudio io.github.zach9113.CrankedAudio1 "$@"; }
state() {
  call GetState | python3 -c "
import json, sys
s = json.loads(json.load(sys.stdin)['data'][0]); k = s['knobs'][0]
print(s['profile'], k['name'], k['level'], 'muted' if k['muted'] else 'unmuted')"
}
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }
timeout 5 dbus-monitor --session "type='signal',interface='io.github.zach9113.CrankedAudio1'" > "$work/signals" 2>&1 &
timeout 5 dbus-monitor --session "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path='/io/github/zach9113/CrankedAudio'" > "$work/property-signals" 2>&1 &
sleep 0.3
check "state before anything" "$(state)" "One Test None unmuted"
call SetKnob uu 0 40 > /dev/null; sleep 0.3
check "SetKnob" "$(state)" "One Test 40 unmuted"
call StepKnob ui 0 -5 > /dev/null; sleep 0.3
check "StepKnob" "$(state)" "One Test 35 unmuted"
call Hold u 0 > /dev/null; sleep 0.9
check "Hold runs the hold action (mute)" "$(state)" "One Test 35 muted"
call SetProfile s Two > /dev/null; sleep 0.3
check "SetProfile" "$(state)" "Two Other 35 unmuted"
# the command line form people bind keys to on any desktop (knobs count from 1 there)
window_bin="$(dirname "$bin")/cranked-audio"
"$window_bin" --knob 1 20; sleep 0.3
check "cranked-audio --knob 1 20" "$(state)" "Two Other 20 unmuted"
"$window_bin" --knob 1 +5; sleep 0.3
check "cranked-audio --knob 1 +5" "$(state)" "Two Other 25 unmuted"
sleep 4
check "StateChanged signals sent" "$([ "$(grep -c 'member=StateChanged' "$work/signals")" -ge 3 ] && echo yes || echo no)" "yes"
# the panel widget follows the State property, which announces every change with PropertiesChanged
property_level=$(busctl --user --json=short get-property -- "$CRANKED_DBUS_NAME" /io/github/zach9113/CrankedAudio \
  io.github.zach9113.CrankedAudio1 State | python3 -c "import json, sys; print(json.loads(json.load(sys.stdin)['data'])['knobs'][0]['level'])")
check "the State property matches GetState" "$property_level" "25"
check "State changes are announced (PropertiesChanged)" \
  "$([ "$(grep -c 'member=PropertiesChanged' "$work/property-signals")" -ge 3 ] && echo yes || echo no)" "yes"
kill $daemon 2>/dev/null; wait 2>/dev/null; rm -rf "$work"
echo "failures: $fails"; exit $fails
