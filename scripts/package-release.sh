#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=$(cargo metadata --manifest-path "$root/Cargo.toml" --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "cranked-audiod") | .version')
tag="v${version}"
arch=$(uname -m)
dist_dir="$root/dist"
pkg_name="cranked-audio-${tag}-${arch}"
pkg_dir="$dist_dir/$pkg_name"

echo "Building release binaries for $pkg_name..."
(cd "$root" && RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=. --remap-path-prefix=$HOME=~" cargo build --release --locked)

echo "Assembling package files..."
rm -rf "$pkg_dir"
mkdir -p "$pkg_dir/bin" "$pkg_dir/share" "$pkg_dir/systemd" "$pkg_dir/udev"

cp "$root/target/release/cranked-audiod" "$pkg_dir/bin/"
cp "$root/target/release/cranked-audio" "$pkg_dir/bin/"
cp "$root/data/cranked-audio.svg" "$pkg_dir/share/"
cp "$root/data/60-cranked-audio.rules" "$pkg_dir/udev/"

# Embedded self-contained installer that requires NO rust/cargo
cat > "$pkg_dir/install.sh" << 'EOF'
#!/usr/bin/env bash
# Standalone Cranked Audio Installer (No Rust toolchain required)
set -euo pipefail

src=$(cd "$(dirname "$0")" && pwd)
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
service="cranked-audio.service"
app="io.github.zach9113.CrankedAudio"

if [ "${1:-}" = "--uninstall" ]; then
  systemctl --user disable --now "$service" 2>/dev/null || true
  rm -f "$bin/cranked-audiod" "$bin/cranked-audio" "$units/$service" \
    "$share/applications/$app.desktop" "$share/icons/hicolor/scalable/apps/$app.svg"
  [ -w /etc/udev/rules.d/60-cranked-audio.rules ] && rm -f /etc/udev/rules.d/60-cranked-audio.rules 2>/dev/null || true
  systemctl --user daemon-reload
  echo "Removed. Your settings in ~/.config/cranked-audio have been preserved."
  exit 0
fi

install -d "$bin" "$share/applications" "$share/icons/hicolor/scalable/apps" "$units"

# Optional udev rule
if [ -w /etc/udev/rules.d ]; then
  install -m 0644 "$src/udev/60-cranked-audio.rules" /etc/udev/rules.d/60-cranked-audio.rules
  command -v udevadm >/dev/null && udevadm control --reload 2>/dev/null || true
fi

install -m 0755 "$src/bin/cranked-audiod" "$bin/cranked-audiod.new" && mv -f "$bin/cranked-audiod.new" "$bin/cranked-audiod"
install -m 0755 "$src/bin/cranked-audio" "$bin/cranked-audio.new" && mv -f "$bin/cranked-audio.new" "$bin/cranked-audio"
install -m 0644 "$src/share/cranked-audio.svg" "$share/icons/hicolor/scalable/apps/$app.svg"

cat > "$share/applications/$app.desktop" << DESKTOP
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
DESKTOP

cat > "$units/$service" << SERVICE
[Unit]
Description=Cranked Audio (volume knobs)
PartOf=graphical-session.target
After=graphical-session.target pipewire-pulse.service

[Service]
ExecStart=$bin/cranked-audiod
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
SERVICE

systemctl --user daemon-reload
systemctl --user enable "$service" >/dev/null
systemctl --user restart --no-block "$service"
command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" 2>/dev/null || true

echo "✓ Cranked Audio installed successfully!"
echo "✓ Background service is active and running."
echo "✓ Open Cranked Audio from your app menu or tray icon."
EOF

chmod +x "$pkg_dir/install.sh"
cp "$root/README.md" "$pkg_dir/"
cp "$root/LICENSE" "$pkg_dir/"

echo "Creating tarball..."
tar -czf "$dist_dir/${pkg_name}.tar.gz" -C "$dist_dir" "$pkg_name"
# the in-app updater refuses a release whose tarball doesn't match this
(cd "$dist_dir" && sha256sum "${pkg_name}.tar.gz" > "${pkg_name}.tar.gz.sha256")
rm -rf "$pkg_dir"

echo "Created: $dist_dir/${pkg_name}.tar.gz"
