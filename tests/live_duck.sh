#!/usr/bin/env bash
# Live Auto-lower check: a "talk" test stream plays silence, a tone, then silence; the "music" test stream should dip
# and come back. Throwaway streams on a null sink; no mixer opened; nothing else touched.
# Usage: tests/live_duck.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
work=$(mktemp -d)
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state"
cat > "$work/config.toml" <<'TOML'
popups = false
[auto_lower]
enabled = true
lower = [1]
triggers = [2]
voice_chat = false
amount = 50
[[channel]]
name = "Music"
apps = ["cranked-selftest-music"]
[[channel]]
name = "Talk"
apps = ["cranked-selftest-talk"]
TOML
module=$(pactl load-module module-null-sink sink_name=cranked_ducktest sink_properties=node.description=cranked_ducktest)
play() { # app name, raw s16 mono 48 kHz file
  pw-play --raw --target cranked_ducktest --rate 48000 --channels 1 --format s16 --volume 1.0 \
    -P "{ application.name=$1 media.role=phone state.restore-props=false }" "$2" &
}
volume_of() {
  pactl -f json list sink-inputs | jq -j --arg app "$1" \
    '.[] | select(.properties."application.name" == $app) | "\((.volume | to_entries[0].value.value) * 100 / 65536 | round)"'
}
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }

head -c $((48000 * 2 * 20)) /dev/zero > "$work/music.raw"
# talk: 0.5 s silence, a 0.3 s ping (like a notification), 1.2 s silence, 1 s tone (440 Hz, about -12 dBFS), 6 s silence
python3 -c "
import math, sys
out = sys.stdout.buffer
tone = lambda seconds: b''.join(int(8000 * math.sin(2 * math.pi * 440 * i / 48000)).to_bytes(2, 'little', signed=True) for i in range(int(48000 * seconds)))
silence = lambda seconds: bytes(int(48000 * seconds) * 2)
out.write(silence(0.5) + tone(0.3) + silence(1.2) + tone(1.0) + silence(6))" > "$work/talk.raw"
play cranked-selftest-music "$work/music.raw"
mkfifo "$work/in"
"$bin" --config "$work/config.toml" --no-mixer --stdin -v < "$work/in" 2> "$work/log" &
daemon=$!
exec 3> "$work/in"
sleep 0.5
echo "knob 0 60" >&3; echo "knob 1 80" >&3; sleep 0.3
play cranked-selftest-talk "$work/talk.raw"
sleep 1.4
check "a short ping on the trigger changes nothing" "$(volume_of cranked-selftest-music)" "60"
sleep 1.75
check "a longer sound lowers the music by half" "$(volume_of cranked-selftest-music)" "30"
pid=$daemon; t0=$(awk '{print $14+$15}' /proc/$pid/stat)
sleep 0.5
check "and holds it down through a short pause" "$(volume_of cranked-selftest-music)" "30"
sleep 2.5
check "then fades back up" "$(volume_of cranked-selftest-music)" "60"
t1=$(awk '{print $14+$15}' /proc/$pid/stat)
echo "cpu while metering (3.5 s): $((t1 - t0)) ticks, rss $(awk '/VmRSS/{print $2" "$3}' /proc/$pid/status)"
grep -q "watching stream" "$work/log" && echo "ok   the trigger stream was metered" || { echo "FAIL no meter"; fails=$((fails + 1)); }
echo "--- daemon log:"; grep -vE "^stream |^channel " "$work/log"
exec 3>&-; kill $daemon 2>/dev/null; kill $(jobs -p) 2>/dev/null; wait 2>/dev/null
pactl unload-module "$module"; rm -rf "$work"
echo "failures: $fails"; exit $fails
