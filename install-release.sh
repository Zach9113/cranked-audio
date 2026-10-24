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
  echo "Error: couldn't find the latest release (GitHub didn't answer, or there are no releases yet)."
  echo "Build from source instead: git clone https://github.com/$REPO && cd cranked-audio && ./install.sh"
  exit 1
fi

TMP_DIR=$(mktemp -d /tmp/cranked-audio-install.XXXXXX)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading Cranked Audio..."
if ! curl -sSL -f "$TARBALL_URL" -o "$TMP_DIR/cranked-audio.tar.gz"; then
  echo "Error: Could not download release tarball from $TARBALL_URL"
  echo "Please check if the release exists or build from source with: git clone https://github.com/$REPO && cd cranked-audio && ./install.sh"
  exit 1
fi

echo "Checking the download..."
if ! curl -sSL -f "$TARBALL_URL.sha256" -o "$TMP_DIR/cranked-audio.tar.gz.sha256"; then
  echo "Error: the release has no checksum file, so it won't be installed."
  exit 1
fi
expected=$(cut -d' ' -f1 < "$TMP_DIR/cranked-audio.tar.gz.sha256")
actual=$(sha256sum "$TMP_DIR/cranked-audio.tar.gz" | cut -d' ' -f1)
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  echo "Error: the download doesn't match the release's checksum; nothing was installed."
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
