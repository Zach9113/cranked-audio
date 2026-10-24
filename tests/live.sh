#!/usr/bin/env bash
# Live check against PipeWire: throwaway silent streams on a null sink, nothing else touched, no mixer opened.
# The streams use media.role=phone so other volume apps' catch-all channels leave them alone.
# Usage: tests/live.sh [path/to/cranked-audiod]
set -u
bin=${1:-target/release/cranked-audiod}
work=$(mktemp -d)
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state"
cat > "$work/config.toml" <<'TOML'
popups = false

[[channel]]
name = "A"
apps = ["cranked-selftest-a"]
press = "play_pause"
hold = "mute"
[[channel]]
name = "B"
apps = ["cranked-selftest-b"]
min = 10
max = 60
TOML
module=$(pactl load-module module-null-sink sink_name=cranked_livetest sink_properties=node.description=cranked_livetest)
play() {
  head -c $((48000 * 2 * 30)) /dev/zero | pw-play --raw --target cranked_livetest --rate 48000 --channels 1 --format s16 \
    --volume 1.0 -P "{ application.name=$1 media.role=phone state.restore-props=false }" - &
}
volume_of() {
  pactl -f json list sink-inputs | jq -j --arg app "$1" \
    '.[] | select(.properties."application.name" == $app) | "\((.volume | to_entries[0].value.value) * 100 / 65536 | round) "'
}
index_of() {
  pactl -f json list sink-inputs | jq --arg app "$1" '[.[] | select(.properties."application.name" == $app)][0].index'
}
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }

muted_of() {
  pactl -f json list sink-inputs | jq -j --arg app "$1" '.[] | select(.properties."application.name" == $app) | "\(.mute) "'
}
play cranked-selftest-a; play cranked-selftest-b
python3 "$(dirname "$0")/fake_player.py" > "$work/player" &
player=$!
sleep 0.5
export CRANKED_MPRIS_PLAYER=org.mpris.MediaPlayer2.cranked_selftest
mkfifo "$work/in"
"$bin" --config "$work/config.toml" --no-mixer --stdin -v < "$work/in" 2> "$work/log" &
daemon=$!
exec 3> "$work/in"
sleep 0.5
echo "knob 0 30" >&3; echo "knob 1 50" >&3; sleep 0.3
check "knob A to 30" "$(volume_of cranked-selftest-a)" "30 "
check "knob B 50 maps onto 10-60" "$(volume_of cranked-selftest-b)" "35 "
pactl set-sink-input-volume "$(index_of cranked-selftest-a)" 90%
sleep 0.1
check "guard puts A back" "$(volume_of cranked-selftest-a)" "30 "
play cranked-selftest-a; sleep 0.6
check "new A stream gets the knob's volume" "$(volume_of cranked-selftest-a)" "30 30 "
for p in $(seq 31 60); do echo "knob 0 $p" >&3; sleep 0.01; done; sleep 0.3
check "fast turn ends on the last value" "$(volume_of cranked-selftest-a)" "60 60 "
echo "down 0" >&3; sleep 0.1; echo "up 0" >&3; sleep 0.4
check "press A sends play/pause to the player" "$(tr '\n' ' ' < "$work/player")" "ready PlayPause "
echo "down 0" >&3; sleep 0.7
check "hold A mutes its streams before the release" "$(muted_of cranked-selftest-a)" "true true "
echo "up 0" >&3; sleep 0.3
play cranked-selftest-a; sleep 0.6
check "a new A stream starts muted" "$(muted_of cranked-selftest-a)" "true true true "
echo "down 0" >&3; sleep 0.7; echo "up 0" >&3; sleep 0.3
check "hold again unmutes" "$(muted_of cranked-selftest-a)" "false false false "
check "B was never muted" "$(muted_of cranked-selftest-b)" "false "
sleep 2
echo "--- idle cost after the test (2 s idle sample):"
t0=$(awk '{print $14+$15}' /proc/$daemon/stat); sleep 2; t1=$(awk '{print $14+$15}' /proc/$daemon/stat)
echo "cpu ticks in 2 s: $((t1 - t0))   rss: $(awk '/VmRSS/{print $2" "$3}' /proc/$daemon/status)   threads: $(ls /proc/$daemon/task | wc -l)"
echo "--- daemon log:"; cat "$work/log"
exec 3>&-; kill $daemon $player 2>/dev/null; kill $(jobs -p) 2>/dev/null; wait 2>/dev/null
pactl unload-module "$module"; rm -rf "$work"
echo "failures: $fails"; exit $fails
