#!/usr/bin/env bash
# The demo video's privacy pass: two frames a second of VIDEO, read with
# Vision, checked for anything that is not the fixture: home and runner
# paths, host and account names, real repositories. Prints every hit and
# exits 1 on any; writes all recognised text to OUT_DIR/ocr.txt for a read.
#
#   scripts/demo-video/privacy-check.sh VIDEO OUT_DIR
set -euo pipefail
VIDEO=${1:?usage: privacy-check.sh VIDEO OUT_DIR}
OUT=${2:?usage: privacy-check.sh VIDEO OUT_DIR}
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$OUT/frames"
rm -f "$OUT"/frames/*.png
ffmpeg -v error -y -i "$VIDEO" -vf fps=2 "$OUT/frames/%04d.png"
[ -x "$OUT/ocr" ] || swiftc -O "$HERE/ocr.swift" -o "$OUT/ocr"
"$OUT/ocr" "$OUT/frames" > "$OUT/ocr.txt"
# Not the fixture: user and runner paths, machine names, the owner's and
# the maintainers' hosts and accounts, the old product name, e-mail.
# (The fixture's own words, like "raw bearer tokens", are not hits.)
pattern='/Users/|/home/[a-z]|\brunner\b|Mac-[0-9]|\.local\b|jiun|june|\brtzr\b|\bmini\b|\bmbp\b|muxa|gemini|@[a-z0-9-]+\.[a-z]{2,}|vault|\b[0-9a-f]{24,}\b|\baas\b'
if grep -inE "$pattern" "$OUT/ocr.txt"; then
  echo "privacy-check: hits above; look at those frames" >&2
  exit 1
fi
echo "privacy-check: $(cut -f2 "$OUT/ocr.txt" | sort -u | wc -l | tr -d ' ') distinct lines, no hits"
