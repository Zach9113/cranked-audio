#!/usr/bin/env bash
# Runs a command inside a throwaway headless session: its own session bus and PipeWire (with the Pulse server and
# WirePlumber), no sound card or desktop needed. CI runs the live suites in it; so can a container or a VM.
# Usage: tests/headless_session.sh tests/live.sh [args...]
set -euo pipefail
if [ -z "${CRANKED_HEADLESS_SESSION:-}" ]; then
  runtime=$(mktemp -d)
  chmod 700 "$runtime"
  # a bus that starts no services on demand: on a desktop the default one would start the real portal and secret
  # service, which outlive the session (and hold its output open)
  cat > "$runtime/bus.conf" <<'XML'
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
XML
  CRANKED_HEADLESS_SESSION=1 XDG_RUNTIME_DIR="$runtime" exec dbus-run-session --config-file="$runtime/bus.conf" -- "$0" "$@"
fi

pipewire &
pipewire-pulse &
wireplumber &
trap 'kill $(jobs -p) 2>/dev/null; wait 2>/dev/null' EXIT
for _ in $(seq 1 50); do
  pactl info >/dev/null 2>&1 && break
  sleep 0.2
done
pactl info >/dev/null || { echo "headless session: PipeWire's Pulse server didn't start" >&2; exit 1; }
# a sink to be the default, as a desktop always has one (the suites add their own null sinks next to it)
# right after start the Pulse server answers before it can load modules ("Not supported"), so try for a while
for _ in $(seq 1 50); do
  pactl load-module module-null-sink sink_name=headless_default >/dev/null 2>&1 && break
  sleep 0.2
done
pactl list short sinks | grep -q headless_default || { echo "headless session: no default sink" >&2; exit 1; }
pactl set-default-sink headless_default
"$@"
