#!/usr/bin/env bash
# Renders data/cranked-audio.svg into the raw RGBA images the tray embeds and the .ico inside the Windows .exe files.
# Run again after changing the icon.
set -eu
cd "$(dirname "$0")"
mkdir -p tray
for size in 22 32 48 64; do
  rsvg-convert -w $size -h $size cranked-audio.svg | magick png:- -depth 8 RGBA:tray/$size.rgba
done
for size in 16 20 24 32 40 48 64 256; do
  rsvg-convert -w $size -h $size cranked-audio.svg > "tray/ico-$size.png"
done
# an .ico of PNG images (Windows reads those for every size; far smaller than ImageMagick's uncompressed ones)
python3 - tray/ico-{16,20,24,32,40,48,64,256}.png > cranked-audio.ico <<'PY'
import struct, sys
images = [open(path, "rb").read() for path in sys.argv[1:]]
sizes = [int(path.rsplit("-", 1)[1].split(".")[0]) for path in sys.argv[1:]]
out = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
for size, data in zip(sizes, images):
    side = 0 if size >= 256 else size
    out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
sys.stdout.buffer.write(out + b"".join(images))
PY
rm tray/ico-*.png
