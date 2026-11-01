#!/usr/bin/env bash
# Checks a release's Linux downloads on whatever system this runs on (scripts/test-channel.sh runs it in clean
# containers): the tarball's checksum, that both programs load, that the service answers a window and scripts, what
# the tarball's install.sh puts where (systemctl is stubbed: containers have no user systemd), that --uninstall
# removes it and keeps the settings, and the AppImage the same way.
# Usage (as a normal user, inside tests/headless_session.sh): tests/channel/release_check.sh DIR_WITH_RELEASE_FILES
set -u
assets=$(cd "${1:?usage: release_check.sh DIR}" && pwd)
work=$(mktemp -d)
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($2)"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails + 1)); fi; }
level() { python3 -c "import json, sys; print(json.load(sys.stdin)['knobs'][0]['level'])"; }

tarball=$(ls "$assets"/cranked-audio-v*-x86_64.tar.gz 2>/dev/null | head -1)
if [ -z "$tarball" ]; then
  echo "FAIL no cranked-audio-v*-x86_64.tar.gz in $assets"
  echo "failures: 1"
  exit 1
fi
(cd "$assets" && sha256sum -c --quiet "$(basename "$tarball").sha256") >/dev/null 2>&1
check "its checksum matches" "$?" "0"
tar xzf "$tarball" -C "$work"
pkg=$(ls -d "$work"/cranked-audio-v*-x86_64)
version=$(basename "$pkg" | sed 's/cranked-audio-\(v[^-]*\)-.*/\1/')

# both programs load here (this system's glibc and libraries are enough)
"$pkg/bin/cranked-audiod" --print-config > /dev/null 2>&1
check "cranked-audiod runs" "$?" "0"

# the service answers, on a test socket
export CRANKED_SOCKET="$work/socket" XDG_STATE_HOME="$work/state" CRANKED_DBUS_NAME="io.github.zach9113.CrankedAudio.Channel$$"
printf 'popups = false\n[[channel]]\nname = "Test"\napps = ["cranked-selftest-none"]\n' > "$work/config.toml"
"$pkg/bin/cranked-audiod" --config "$work/config.toml" --no-mixer > "$work/log" 2>&1 &
daemon=$!
sleep 2
check "the version is in the file name" "$(case "$version" in v[0-9]*) echo yes ;; *) echo "$version" ;; esac)" "yes"
check "the service reports its version" "$("$pkg/bin/cranked-audio" --state | python3 -c "import json, sys; print(json.load(sys.stdin)['version'])")" "$version"
# a program built against a newer glibc may still run but print loader errors ("version GLIBC_2.39 not found")
check "no loader errors on this system" "$("$pkg/bin/cranked-audio" --state 2>&1 >/dev/null | head -1)" ""
"$pkg/bin/cranked-audio" --knob 1 40; sleep 0.5
check "a knob command reaches it" "$("$pkg/bin/cranked-audio" --state | level)" "40"
kill $daemon 2>/dev/null; wait $daemon 2>/dev/null

# the tarball's installer, with systemctl recorded instead of run
mkdir -p "$work/stub"
printf '#!/bin/sh\necho "$*" >> "%s/systemctl.log"\nexit 0\n' "$work" > "$work/stub/systemctl"
chmod +x "$work/stub/systemctl"
mkdir -p "$HOME/.config/cranked-audio" && echo "# kept" > "$HOME/.config/cranked-audio/config.toml"
PATH="$work/stub:$PATH" "$pkg/install.sh" > "$work/install.log" 2>&1
check "install.sh succeeds" "$?" "0"
for f in .local/bin/cranked-audiod .local/bin/cranked-audio .config/systemd/user/cranked-audio.service \
  .local/share/applications/io.github.zach9113.CrankedAudio.desktop \
  .config/autostart/io.github.zach9113.CrankedAudio.Autostart.desktop; do
  check "installed ~/$f" "$([ -e "$HOME/$f" ] && echo yes || echo no)" "yes"
done
check "the service is enabled" "$(grep -c '^--user enable cranked-audio.service' "$work/systemctl.log")" "1"
PATH="$work/stub:$PATH" "$pkg/install.sh" --uninstall > /dev/null 2>&1
check "--uninstall removes the programs" "$([ -e "$HOME/.local/bin/cranked-audiod" ] && echo no || echo yes)" "yes"
check "--uninstall keeps the settings" "$(cat "$HOME/.config/cranked-audio/config.toml")" "# kept"

# the AppImage: its window program and (with --daemon) its service, unpacked to run (no FUSE in containers)
appimage="$assets/CrankedAudio-x86_64.AppImage"
if [ -e "$appimage" ]; then
  cp "$appimage" "$work/app.AppImage" && chmod +x "$work/app.AppImage"
  export APPIMAGE_EXTRACT_AND_RUN=1
  "$work/app.AppImage" --daemon --config "$work/config.toml" --no-mixer > "$work/appimage.log" 2>&1 &
  daemon=$!
  sleep 3
  "$work/app.AppImage" --knob 1 25; sleep 0.5
  check "the AppImage's service and window program work" "$("$work/app.AppImage" --state | level)" "25"
  kill $daemon 2>/dev/null; wait $daemon 2>/dev/null
else
  echo "skip the AppImage (not in $assets)"
fi
rm -rf "$work"
echo "failures: $fails"
exit $fails
