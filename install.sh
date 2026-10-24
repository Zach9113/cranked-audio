#!/usr/bin/env bash
# Installs Cranked Audio for the current user (no root needed):
#   ~/.local/bin/cranked-audiod, ~/.local/bin/cranked-audio, an app-menu entry, the icon, and a systemd user service
#   that starts it at login.
# Usage: ./install.sh            build (release) and install, then start it
#        ./install.sh --uninstall stop it and remove everything this installed (settings are kept)
set -euo pipefail

root=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
service=cranked-audio.service
app=io.github.zach9113.CrankedAudio

if [ "${1:-}" = "--uninstall" ]; then
  systemctl --user disable --now "$service" 2>/dev/null || true
  rm -f "$bin/cranked-audiod" "$bin/cranked-audio" "$units/$service" \
    "$share/applications/$app.desktop" "$share/icons/hicolor/scalable/apps/$app.svg"
  [ -w /etc/udev/rules.d/60-cranked-audio.rules ] && rm -f /etc/udev/rules.d/60-cranked-audio.rules 2>/dev/null || true
  systemctl --user daemon-reload
  echo "Removed. Your settings are still in ~/.config/cranked-audio."
  exit 0
fi

(cd "$root" && RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=. --remap-path-prefix=$HOME=~" cargo build --release --locked)

install -d "$bin" "$share/applications" "$share/icons/hicolor/scalable/apps" "$units"
# udev rule for mixer permissions without requiring group changes (installed if writable)
if [ -w /etc/udev/rules.d ]; then
  install -m 0644 "$root/data/60-cranked-audio.rules" /etc/udev/rules.d/60-cranked-audio.rules
  command -v udevadm >/dev/null && udevadm control --reload 2>/dev/null || true
fi
# replace the binaries without disturbing a running copy (it keeps its old file until it restarts)
install -m 0755 "$root/target/release/cranked-audiod" "$bin/cranked-audiod.new" && mv -f "$bin/cranked-audiod.new" "$bin/cranked-audiod"
install -m 0755 "$root/target/release/cranked-audio" "$bin/cranked-audio.new" && mv -f "$bin/cranked-audio.new" "$bin/cranked-audio"
install -m 0644 "$root/data/cranked-audio.svg" "$share/icons/hicolor/scalable/apps/$app.svg"

cat > "$share/applications/$app.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Cranked Audio
GenericName=Volume Knobs
Comment=Per-app volume knobs, on a USB mixer or on screen
Exec=$bin/cranked-audio
Icon=$app
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=volume;mixer;knob;audio;controller;
StartupWMClass=$app
EOF

cat > "$units/$service" <<EOF
[Unit]
Description=Cranked Audio (volume knobs)
PartOf=graphical-session.target
After=graphical-session.target pipewire-pulse.service

[Service]
ExecStart=$bin/cranked-audiod
# quitting from the tray is a clean exit and stays quit; a crash comes back
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF

systemctl --user daemon-reload
systemctl --user enable "$service" >/dev/null
systemctl --user restart --no-block "$service"
command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" 2>/dev/null || true
echo "Installed. Cranked Audio is running and starts at login; open it from the app menu or the tray."
