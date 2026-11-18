#!/usr/bin/env bash
# Capture the landing page's dashboard screenshots from a real tend.
#
#   scripts/landing-shots/live.sh
#
# Stands up the demo GIFs' fixture fleet (docs/demo-setup.sh on the
# isolated sandbox) with the dashboard on, adds two results waiting for a
# decision, and has capture-live.mjs photograph Home, Explore and the
# workbench's collaboration panel in light and dark. Everything shown is
# fixture; nothing reads the tend you use.
#
# Binaries: TEND_SHOTS_BIN_DIR (default target/debug; on macOS point it at
# wrappers from wrap-bins.sh). Port: TEND_SHOTS_PORT (default 7993, refused
# if something already listens). Needs tmux, node with playwright-core
# (TEND_PLAYWRIGHT_PACKAGE), Chrome and cwebp. Teardown runs on exit.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="${TEND_SHOTS_BIN_DIR:-$REPO/target/debug}"
PORT="${TEND_SHOTS_PORT:-7993}"

if curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" 2>/dev/null; then
  echo "live.sh: something already listens on 127.0.0.1:$PORT; set TEND_SHOTS_PORT" >&2
  exit 1
fi

TOKEN=$(openssl rand -hex 24)
cleanup() { bash "$REPO/docs/demo-teardown.sh" >/dev/null 2>&1 || true; }
trap cleanup EXIT

env -u TMUX -u TMUX_PANE \
  TEND_DEMO_TMUX="${TEND_DEMO_TMUX:-$(command -v tmux)}" \
  TEND_DEMO_BIN_DIR="$BIN_DIR" \
  TEND_DEMO_DASHBOARD="127.0.0.1:$PORT" \
  TEND_DEMO_DASHBOARD_TOKEN="$TOKEN" \
  bash "$REPO/docs/demo-setup.sh" >/dev/null

eval "$(bash "$REPO/scripts/tend-sandbox.sh" env --name tend-demo)"
tend() { env -u TMUX -u TMUX_PANE "$BIN_DIR/tend" "$@"; }

# Two finished results for Home's Your turn, filed the way an agent files one.
tend review request --session s-billing --as agent --label billing \
  --summary "Proration now settles a plan that changes twice in one period" \
  --decision-needed "Merge the proration fix?" \
  --option "merge" --option "wait for finance" \
  --recommend "merge: the two-change case has a test now" \
  --evidence "test-pass:cargo test -p billing proration" >/dev/null
tend review request --session s-data --as agent --label data \
  --summary "Analytics rollups backfilled for last week" \
  --decision-needed "Publish the backfilled rollups?" \
  --option "publish" --option "hold" \
  --recommend "publish: 14 models ran with 0 errors" >/dev/null

waited=0
until curl -fsS -o /dev/null -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/home"; do
  waited=$((waited + 1)); [ "$waited" -lt 50 ] || { echo "live.sh: dashboard never came up" >&2; exit 1; }
  sleep 0.2
done

TEND_SHOTS_URL="http://127.0.0.1:$PORT" TEND_SHOTS_TOKEN="$TOKEN" node "$HERE/capture-live.mjs"
