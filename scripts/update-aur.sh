#!/usr/bin/env bash
# Points the AUR package at a published release: sets pkgver, resets pkgrel, fills in both checksums and
# regenerates .SRCINFO. Then copy PKGBUILD, .SRCINFO and cranked-audio.install to the AUR checkout and push.
# Usage: scripts/update-aur.sh v2.3.0
set -euo pipefail

tag=${1:?usage: scripts/update-aur.sh vX.Y.Z}
version=${tag#v}
dir=$(cd "$(dirname "$0")/../packaging/aur/cranked-audio-bin" && pwd)
url=https://github.com/Zach9113/cranked-audio

# the release's own .sha256 file, so the package checks against what CI published
tarball_sum=$(curl -fsSL "$url/releases/download/$tag/cranked-audio-$tag-x86_64.tar.gz.sha256" | cut -d' ' -f1)
license_sum=$(curl -fsSL "https://raw.githubusercontent.com/Zach9113/cranked-audio/$tag/LICENSE" | sha256sum | cut -d' ' -f1)
[[ $tarball_sum =~ ^[0-9a-f]{64}$ ]] || { echo "no checksum published for $tag" >&2; exit 1; }

sed -i -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "/^sha256sums=/,/)/c\\sha256sums=('$tarball_sum'\\n            '$license_sum')" "$dir/PKGBUILD"
(cd "$dir" && makepkg --printsrcinfo > .SRCINFO)
echo "PKGBUILD and .SRCINFO now point at $tag."
