#!/usr/bin/env bash
#
# Install both Cranked Audio and the Atoll Dynamic Island (with in-island mixer)
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-both.sh | bash
#   or: ./install-both.sh
#

set -euo pipefail

BOLD='\033[1m'
BLUE='\033[1;34m'
GREEN='\033[1;32m'
WARN='\033[1;33m'
OFF='\033[0m'
if [ ! -t 1 ]; then
    BOLD='' BLUE='' GREEN='' WARN='' OFF=''
fi

say()  { printf '%b==>%b %b%s%b\n' "$BLUE" "$OFF" "$BOLD" "$1" "$OFF"; }
good() { printf '%b==>%b %b%s%b\n' "$GREEN" "$OFF" "$BOLD" "$1" "$OFF"; }
warn() { printf '%b!!%b  %s\n' "$WARN" "$OFF" "$1"; }
die()  { printf '%b!!%b  %s\n' "$WARN" "$OFF" "$1" >&2; exit 1; }

[ "$(id -u)" -ne 0 ] || die "Do not run this script as root/sudo. It will ask for your sudo password only when installing to /usr."

say "Cranked Audio + Atoll Dynamic Island Bundle Installer"
echo "This will install:"
echo "  1. Cranked Audio (per-app volume mixer daemon & tray)"
echo "  2. Atoll Dynamic Island (Zach9113 fork with embedded mixer & performance optimizations)"
echo ""

# -----------------------------------------------------------------------------
# 1. Cranked Audio
# -----------------------------------------------------------------------------
say "Step 1/2: Installing Cranked Audio..."
curl -fsSL https://raw.githubusercontent.com/Zach9113/cranked-audio/main/install-release.sh | bash

# -----------------------------------------------------------------------------
# 2. Atoll Dynamic Island
# -----------------------------------------------------------------------------
say "Step 2/2: Installing Atoll (zach-patches)..."

ATOLL_TMP=$(mktemp -d /tmp/atoll-bundle-install.XXXXXX)
trap 'rm -rf "$ATOLL_TMP"' EXIT

say "Fetching Atoll source..."
git clone --depth 1 --branch zach-patches https://github.com/Zach9113/atoll.git "$ATOLL_TMP/atoll"

if command -v pacman >/dev/null 2>&1 && command -v makepkg >/dev/null 2>&1; then
    # Arch and friends: build the fork's package, so pacman owns the files and can update or remove them later.
    if [ "$(pacman -Qq atoll 2>/dev/null)" = "atoll" ]; then
        die "The stock 'atoll' package is installed. Remove it first (sudo pacman -R atoll), then run this again."
    fi
    say "Installing build tools via pacman..."
    sudo pacman -S --needed --noconfirm base-devel git
    say "Building and installing the atoll-git package..."
    cd "$ATOLL_TMP/atoll/packaging"
    cp PKGBUILD-git PKGBUILD
    makepkg -si --noconfirm
else
    warn "No pacman/makepkg here: building with CMake and installing into /usr (install the Qt 6, layer-shell-qt,"
    warn "KF6 CoreAddons/I18n and Wayland development packages first if the build fails)."
    command -v sudo >/dev/null 2>&1 || die "This installer requires 'sudo' to install Atoll into /usr."
    cd "$ATOLL_TMP/atoll"
    say "Building Atoll..."
    cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
    cmake --build build
    say "Installing Atoll into /usr..."
    sudo cmake --install build
fi

say "Starting Atoll user service..."
systemctl --user daemon-reload
systemctl --user enable --now atoll.service

# -----------------------------------------------------------------------------
# Done!
# -----------------------------------------------------------------------------
echo ""
good "Installation complete!"
echo ""
echo "Both services start automatically at login. Right now:"
echo "  - Cranked Audio : $(systemctl --user is-active cranked-audio.service 2>/dev/null || true)"
echo "  - Atoll Island  : $(systemctl --user is-active atoll.service 2>/dev/null || true)"
echo ""
echo "Click the dynamic island at the top of your screen to expand it, then"
echo "click 'Mixer' to see your live Cranked Audio knobs inside the island!"
