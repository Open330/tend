#!/usr/bin/env bash
# The demo video's privacy pass: two frames a second of VIDEO (sampled, not
# every frame), read with
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
# Not the fixture: user and runner paths, machine names, e-mail, long hex
# (tokens, ids). The fixture's own words, like "raw bearer tokens", are not
# hits. Names that are private (your hosts, accounts, people) do not belong
# in this file: put one extended regex per line in a file outside the
# repository and pass it as PRIVACY_PATTERNS.
pattern='/Users/|/home/[a-z]|\brunner\b|Mac-[0-9]|\.local\b|@[a-z0-9-]+\.[a-z]{2,}|\b[0-9a-f]{24,}\b'
# Each pattern is its own -e, so a private file cannot change the others'
# meaning; an empty file adds nothing.
args=(-e "$pattern")
if [ -n "${PRIVACY_PATTERNS:-}" ]; then
  [ -r "$PRIVACY_PATTERNS" ] || { echo "privacy-check: cannot read $PRIVACY_PATTERNS" >&2; exit 2; }
  while IFS= read -r line || [ -n "$line" ]; do
    [[ "$line" =~ ^[[:space:]]*$ ]] || args+=(-e "$line")
  done < "$PRIVACY_PATTERNS"
fi
# grep: 0 = hits, 1 = clean, anything else (a bad pattern, an unreadable
# file) is a failed check, never a pass.
set +e
grep -inE "${args[@]}" "$OUT/ocr.txt"
status=$?
set -e
case $status in
  0) echo "privacy-check: hits above; look at those frames" >&2; exit 1 ;;
  1) ;;
  *) echo "privacy-check: the scan itself failed (grep status $status)" >&2; exit 2 ;;
esac
echo "privacy-check: $(cut -f2 "$OUT/ocr.txt" | sort -u | wc -l | tr -d ' ') distinct lines, no hits"
