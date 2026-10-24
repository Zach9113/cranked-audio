#!/usr/bin/env bash
# Builds CrankedAudio-x86_64.AppImage
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
dist_dir="$root/dist"
appdir="$dist_dir/CrankedAudio.AppDir"
cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/cranked-audio-tools"

echo "Building release binaries..."
(cd "$root" && RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=. --remap-path-prefix=$HOME=~" cargo build --release --locked)

echo "Assembling AppDir at $appdir..."
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/icons/hicolor/scalable/apps" "$appdir/usr/share/applications"

cp "$root/target/release/cranked-audiod" "$appdir/usr/bin/"
cp "$root/target/release/cranked-audio" "$appdir/usr/bin/"
cp "$root/data/cranked-audio.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/io.github.zach9113.CrankedAudio.svg"
cp "$root/data/cranked-audio.svg" "$appdir/io.github.zach9113.CrankedAudio.svg"
ln -sf "io.github.zach9113.CrankedAudio.svg" "$appdir/.DirIcon"
ln -sf "io.github.zach9113.CrankedAudio.svg" "$appdir/cranked-audio.svg"

cat > "$appdir/io.github.zach9113.CrankedAudio.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=Cranked Audio
GenericName=Volume Knobs
Comment=Per-app volume knobs, on a USB mixer or on screen
Exec=cranked-audio
Icon=io.github.zach9113.CrankedAudio
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=volume;mixer;knob;audio;controller;
StartupWMClass=io.github.zach9113.CrankedAudio
EOF
ln -sf "io.github.zach9113.CrankedAudio.desktop" "$appdir/cranked-audio.desktop"

cat > "$appdir/AppRun" << 'EOF'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

if [ "${1:-}" = "--daemon" ]; then
  exec "$HERE/usr/bin/cranked-audiod" "${@:2}"
fi

exec "$HERE/usr/bin/cranked-audio" "$@"
EOF
chmod +x "$appdir/AppRun"

echo "Ensuring appimagetool is available..."
APPIMAGETOOL="$(command -v appimagetool || true)"
if [ -z "$APPIMAGETOOL" ]; then
  mkdir -p "$cache_dir"
  APPIMAGETOOL="$cache_dir/appimagetool"
  if [ ! -x "$APPIMAGETOOL" ]; then
    echo "Downloading appimagetool..."
    curl -sSL -f -o "$APPIMAGETOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x "$APPIMAGETOOL"
  fi
fi

echo "Creating AppImage..."
export ARCH=x86_64
"$APPIMAGETOOL" "$appdir" "$dist_dir/CrankedAudio-x86_64.AppImage"
rm -rf "$appdir"

echo "AppImage created: $dist_dir/CrankedAudio-x86_64.AppImage"
