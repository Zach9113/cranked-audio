#!/usr/bin/env bash
# Renders data/cranked-audio.svg into the raw RGBA images the tray embeds. Run again after changing the icon.
set -eu
cd "$(dirname "$0")"
mkdir -p tray
for size in 22 32 48 64; do
  rsvg-convert -w $size -h $size cranked-audio.svg | magick png:- -depth 8 RGBA:tray/$size.rgba
done
