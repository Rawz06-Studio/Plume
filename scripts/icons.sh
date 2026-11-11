#!/usr/bin/env bash
# Regenerates every icon format from assets/icon.svg.
# Requires rsvg-convert (brew install librsvg), plus iconutil on macOS for the .icns.
set -euo pipefail
cd "$(dirname "$0")/.."

svg=assets/icon.svg
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

render() { rsvg-convert -w "$1" -h "$1" "$svg" -o "$2"; }

# PNG used for the window / dock icon at runtime
render 256 assets/icon-256.png

# macOS .icns
set_dir="$tmp/icon.iconset"
mkdir -p "$set_dir"
for s in 16 32 128 256 512; do
  render "$s" "$set_dir/icon_${s}x${s}.png"
  render $((s * 2)) "$set_dir/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$set_dir" -o assets/icon.icns

# Windows .ico (PNG-compressed entries)
for s in 16 24 32 48 64 128 256; do render "$s" "$tmp/$s.png"; done
python3 - "$tmp" assets/icon.ico <<'PY'
import struct, sys
src, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
blobs = [open(f"{src}/{s}.png", "rb").read() for s in sizes]
offset = 6 + 16 * len(sizes)
data = struct.pack("<HHH", 0, 1, len(sizes))
for s, b in zip(sizes, blobs):
    dim = 0 if s == 256 else s
    data += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(b), offset)
    offset += len(b)
open(out, "wb").write(data + b"".join(blobs))
PY

echo "icons generated in assets/"
