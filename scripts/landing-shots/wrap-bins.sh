#!/usr/bin/env bash
# Put sandbox-exec wrappers for tend and tendd in a directory, for the
# recordings and screenshots on macOS.
#
#   scripts/landing-shots/wrap-bins.sh <real-bin-dir> <wrapper-dir>
#
# tendd always scans the default tmux directory (/private/tmp/tmux-<uid>),
# so a fixture daemon would otherwise see, and write its socket into, the
# tmux server you actually use. The wrappers deny that directory, as
# apps/tend-ios/Scripts/scratch-tendd.sh does. On Linux they just exec.
#
# Point TEND_DEMO_BIN_DIR (docs/demo-setup.sh) or the tapes' PATH at the
# wrapper directory.
set -euo pipefail

real=$(cd "${1:?real binary dir}" && pwd)
out=${2:?wrapper dir}
mkdir -p -m 700 "$out"
out=$(cd "$out" && pwd)
uid=$(id -u)
cat > "$out/profile.sb" <<SB
(version 1)
(allow default)
(deny file-read* file-write* (subpath "/private/tmp/tmux-$uid"))
(deny network-outbound (remote unix-socket (subpath "/private/tmp/tmux-$uid")))
(allow process-exec (literal "/bin/ps") (with no-sandbox))
SB
for bin in tend tendd; do
  [ -x "$real/$bin" ] || { echo "wrap-bins: $real/$bin is missing" >&2; exit 1; }
  if [ "$(uname)" = Darwin ]; then
    printf '#!/bin/sh\nexec /usr/bin/sandbox-exec -f %s %s "$@"\n' "$out/profile.sb" "$real/$bin" > "$out/$bin"
  else
    printf '#!/bin/sh\nexec %s "$@"\n' "$real/$bin" > "$out/$bin"
  fi
  chmod +x "$out/$bin"
done
