#!/usr/bin/env bash
# Live check of the extra devices with virtual ones (nothing real is touched):
#   a keyboard knob (a virtual input device with volume keys, taken over so the system volume must NOT move),
#   a MIDI controller (a virtual MIDI port), and a deej slider box (a virtual serial cable).
# Each drives a knob whose throwaway stream sits on a null sink.
# Usage: tests/live_devices.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state" CRANKED_DBUS_NAME="io.github.zach9113.CrankedAudio.Dev$$"
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }

module=$(pactl load-module module-null-sink sink_name=cranked_devtest sink_properties=node.description=cranked_devtest)
for app in kb midi deej; do
  head -c $((48000 * 2 * 40)) /dev/zero > "$work/$app.raw"
  pw-play --raw --target cranked_devtest --rate 48000 --channels 1 --format s16 --volume 1.0 \
    -P "{ application.name=cranked-selftest-$app media.role=phone state.restore-props=false }" "$work/$app.raw" &
done
stream() {
  pactl -f json list sink-inputs | jq -j --arg app "cranked-selftest-$1" \
    '.[] | select(.properties."application.name" == $app) | "\((.volume | to_entries[0].value.value) * 100 / 65536 | round)\(if .mute then " muted" else "" end)"'
}
system_volume() { pactl get-sink-volume @DEFAULT_SINK@ | grep -o '[0-9]*%' | head -1; }

# the virtual devices
socat pty,raw,echo=0,link="$work/deej-box" pty,raw,echo=0,link="$work/deej-us" &
mkfifo "$work/keys" "$work/midi"
python3 - "$work/keys" <<'PY' &
import sys, time
from evdev import UInput, ecodes as e
ui = UInput({e.EV_KEY: [e.KEY_VOLUMEUP, e.KEY_VOLUMEDOWN, e.KEY_MUTE, e.KEY_PLAYPAUSE]}, name="Cranked Test Knob")
for line in open(sys.argv[1]):
    key = getattr(e, line.strip())
    ui.write(e.EV_KEY, key, 1); ui.syn(); time.sleep(0.03)
    ui.write(e.EV_KEY, key, 0); ui.syn()
ui.close()
PY
"$here/../target/release/examples/midi_virtual" < "$work/midi" &
exec 3> "$work/keys" 4> "$work/midi"
sleep 0.5

cat > "$work/config.toml" <<TOML
popups = false
[[device]]
kind = "keyboard_knob"
name = "Cranked Test Knob"
knob = 1
step = 5
[[device]]
kind = "midi"
name = "Cranked Test MIDI"
controls = [
    { channel = 1, kind = "cc", number = 7, knob = 2 },
    { channel = 1, kind = "cc", number = 64, knob = 2, button = true },
]
[[device]]
kind = "deej"
port = "$work/deej-box"
knobs = [3]
[[channel]]
name = "KB"
apps = ["cranked-selftest-kb"]
press = "mute"
hold = "none"
[[channel]]
name = "MIDI"
apps = ["cranked-selftest-midi"]
press = "mute"
hold = "none"
[[channel]]
name = "DEEJ"
apps = ["cranked-selftest-deej"]
TOML
before=$(system_volume)
"$bin" --config "$work/config.toml" --no-mixer -v > "$work/log" 2>&1 &
daemon=$!
# the service looks for devices every 3 s; give it time to find and take over the keyboard knob
sleep 4.5

echo KEY_VOLUMEUP >&3; echo KEY_VOLUMEUP >&3; echo KEY_VOLUMEUP >&3; sleep 0.6
check "keyboard knob turns its knob up (3 x 5%)" "$(stream kb)" "15"
echo KEY_VOLUMEDOWN >&3; sleep 0.5
check "and down" "$(stream kb)" "10"
echo KEY_MUTE >&3; sleep 0.6
check "pressing it is the knob's button (mute)" "$(stream kb)" "10 muted"
after=$(system_volume)
check "the system volume didn't move (the knob is taken over)" "$after" "$before"
[ "$after" != "$before" ] && pactl set-sink-volume @DEFAULT_SINK@ "$before"

echo "b0 07 40" >&4; sleep 0.5
check "MIDI fader sets its knob" "$(stream midi)" "50"
echo "b0 07 7f" >&4; sleep 0.5
check "all the way up" "$(stream midi)" "100"
echo "b0 40 7f" >&4; sleep 0.1; echo "b0 40 00" >&4; sleep 0.5
check "MIDI button is the knob's button (mute)" "$(stream midi)" "100 muted"

for _ in 1 2 3 4 5; do printf '512\r\n' > "$work/deej-us"; sleep 0.1; done; sleep 0.4
check "deej slider sets its knob" "$(stream deej)" "50"
for _ in 1 2 3 4 5; do printf '1023\r\n' > "$work/deej-us"; sleep 0.1; done; sleep 0.4
check "all the way up" "$(stream deej)" "100"

echo "--- service log:"; grep -E "keyboard knob|midi|deej" "$work/log" | head -6
exec 3>&- 4>&-
kill $daemon 2>/dev/null; kill $(jobs -p) 2>/dev/null; wait 2>/dev/null
pactl unload-module "$module"; rm -rf "$work"
echo "failures: $fails"; exit $fails
