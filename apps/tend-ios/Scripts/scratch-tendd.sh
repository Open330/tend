#!/usr/bin/env bash
# A scratch tendd with the dashboard on 127.0.0.1, seeded with one question
# and one result to review, for the iOS app's simulator and UI tests.
#
#   apps/tend-ios/Scripts/scratch-tendd.sh up      # build nothing; start and seed
#   apps/tend-ios/Scripts/scratch-tendd.sh seed    # add another question + result
#   apps/tend-ios/Scripts/scratch-tendd.sh down
#
# It never touches the tend you use: everything runs through
# scripts/tend-sandbox.sh (private socket, config, data dir and tmux server),
# and on macOS the daemon and CLI also run under sandbox-exec with the
# default tmux directory (/private/tmp/tmux-<uid>) denied, because tendd
# always scans that directory and a scratch daemon would otherwise write
# its socket into the real server's environment.
#
# Binaries: $TENDD and $TEND (default target/debug). Port: $PORT (7979).
# The operator token is printed by `up` and stored in the sandbox root.
set -euo pipefail

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
NAME=tend-ios
ROOT=/tmp/$NAME-sandbox
PORT=${PORT:-7979}
TENDD=${TENDD:-$REPO/target/debug/tendd}
TEND=${TEND:-$REPO/target/debug/tend}
WRAP=/tmp/$NAME-wrap
SANDBOX=$REPO/scripts/tend-sandbox.sh

die() { printf 'scratch-tendd: %s\n' "$*" >&2; exit 1; }

wrappers() {
  mkdir -p -m 700 "$WRAP"
  local uid; uid=$(id -u)
  cat > "$WRAP/profile.sb" <<SB
(version 1)
(allow default)
(deny file-read* file-write* (subpath "/private/tmp/tmux-$uid"))
(deny network-outbound (remote unix-socket (subpath "/private/tmp/tmux-$uid")))
(allow process-exec (literal "/bin/ps") (with no-sandbox))
SB
  for bin in tendd tend; do
    local real; [ "$bin" = tendd ] && real=$TENDD || real=$TEND
    [ -x "$real" ] || die "$bin not found at $real (build it, or set TENDD / TEND)"
    if [ "$(uname)" = Darwin ]; then
      printf '#!/bin/sh\nexec /usr/bin/sandbox-exec -f %s %s "$@"\n' "$WRAP/profile.sb" "$real" > "$WRAP/$bin"
    else
      printf '#!/bin/sh\nexec %s "$@"\n' "$real" > "$WRAP/$bin"
    fi
    chmod +x "$WRAP/$bin"
  done
}

token() { cat "$ROOT/operator-token"; }

seed() {
  eval "$("$SANDBOX" env --name "$NAME")"
  local n; n=$(date +%s)
  # A question for the person, asked from an agent pane in the sandbox.
  tmux new-window -d -n "asker-$n" \
    "printf '%s' '{\"session_id\":\"ios-asker-$n\",\"prompt\":\"migrate storage\"}' | $WRAP/tend hook claude --event user_prompt_submit; $WRAP/tend msg attention 'Ship the storage migration behind a flag? ($n)' --action choice; sleep 600"
  # A result filed for review, with the decision context the card shows.
  "$WRAP/tend" review request --session "ios-scratch-$n" --as agent --label "impl" \
    --summary "Retry path for the storage migration ($n)" \
    --decision-needed "Merge the retry path now?" \
    --option "merge now" --option "wait for the perf run" \
    --recommend "merge now: the perf run only covers reads" \
    --evidence "https://github.com/Open330/tend/pull/1" \
    --evidence "test-pass:cargo test -p tend review" >/dev/null
  local waited=0
  until curl -fsS -H "Authorization: Bearer $(token)" "http://127.0.0.1:$PORT/api/decisions" \
      | grep -q "flag? ($n)" ; do
    waited=$((waited + 1)); [ "$waited" -lt 50 ] || die "the seeded question never reached the queue"
    sleep 0.2
  done
}

up() {
  [ -n "${TMUX:-}" ] && ALLOW=--allow-inside-tmux || ALLOW=
  wrappers
  local cfg="$WRAP/config.toml" tok
  tok=$(openssl rand -hex 24)
  cat > "$cfg" <<TOML
[discovery]
enabled = false

[reconciler]
codex_rollout_enabled = false

[collaboration]
enabled = true
wake = 'never'
scope = 'host'

[dashboard]
enabled = true
bind = "127.0.0.1:$PORT"
token = "$tok"
TOML
  env -u TMUX -u TMUX_PANE "$SANDBOX" up --name "$NAME" --config "$cfg" \
    --tendd "$WRAP/tendd" --tend "$WRAP/tend" --extra-path "$WRAP" ${ALLOW:+$ALLOW}
  printf '%s\n' "$tok" > "$ROOT/operator-token"
  chmod 600 "$ROOT/operator-token"
  env -u TMUX -u TMUX_PANE "$SANDBOX" daemon --name "$NAME" --tendd "$WRAP/tendd" --tend "$WRAP/tend"
  local waited=0
  until curl -fsS -o /dev/null "http://127.0.0.1:$PORT/auth/session"; do
    waited=$((waited + 1)); [ "$waited" -lt 50 ] || die "the dashboard did not come up on $PORT"
    sleep 0.2
  done
  seed
  echo "server=http://127.0.0.1:$PORT"
  echo "token=$tok"
}

down() {
  env -u TMUX -u TMUX_PANE "$SANDBOX" down --name "$NAME" --tendd "${WRAP}/tendd" --tend "${WRAP}/tend" || true
}

case "${1:-}" in
  up) up ;;
  seed) seed ;;
  token) token ;;
  down) down ;;
  *) die "usage: $0 up|seed|token|down" ;;
esac
