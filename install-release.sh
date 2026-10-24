#!/usr/bin/env bash
# One-line installer for Cranked Audio prebuilt release
set -euo pipefail

REPO="Zach9113/cranked-audio"
ARCH="$(uname -m)"

if [ "$ARCH" != "x86_64" ]; then
  echo "Error: Cranked Audio currently provides prebuilt releases for x86_64 (detected: $ARCH)."
  echo "You can build from source by cloning the repository and running ./install.sh"
  exit 1
fi

echo "Fetching latest Cranked Audio release..."
RELEASE_JSON=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || echo "{}")
TARBALL_URL=$(echo "$RELEASE_JSON" | grep -o 'https://github.com/[^"]*cranked-audio-v[^"]*-x86_64\.tar\.gz' | head -n1 || true)

if [ -z "$TARBALL_URL" ]; then
  # Fallback to latest tag asset url if API rate-limited or private
  TARBALL_URL="https://github.com/${REPO}/releases/latest/download/cranked-audio-v0.1.0-x86_64.tar.gz"
fi

TMP_DIR=$(mktemp -d /tmp/cranked-audio-install.XXXXXX)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading Cranked Audio..."
if ! curl -sSL -f "$TARBALL_URL" -o "$TMP_DIR/cranked-audio.tar.gz"; then
  echo "Error: Could not download release tarball from $TARBALL_URL"
  echo "Please check if the release exists or build from source with: git clone https://github.com/$REPO && cd cranked-audio && ./install.sh"
  exit 1
fi

echo "Extracting..."
tar -xzf "$TMP_DIR/cranked-audio.tar.gz" -C "$TMP_DIR"

PKG_DIR=$(find "$TMP_DIR" -maxdepth 1 -mindepth 1 -type d -name "cranked-audio*" | head -n 1)
if [ -z "$PKG_DIR" ] || [ ! -f "$PKG_DIR/install.sh" ]; then
  echo "Error: failed to locate package installer inside release tarball."
  exit 1
fi

"$PKG_DIR/install.sh" "$@"
