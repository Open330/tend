#!/usr/bin/env bash
# Record the Tend for Mac part of the product demo video.
#
#   scripts/demo-video/record-mac.sh APP OUT_DIR
#
# CI only (.github/workflows/demo-video.yml): it changes the runner's display
# mode, Dock and wallpaper, launches the app on screen and drives the real
# mouse. Everything the app shows is the demo fixture (docs/demo-setup.sh on
# the tend-demo sandbox, with its dashboard on 127.0.0.1:$PORT for the iOS
# part, which runs afterwards against the same fleet).
#
# Writes OUT_DIR/mac-raw.mp4 (1920x1080, 30 fps, with the cursor),
# OUT_DIR/marks.txt (`<scene> <seconds into the take>`), a screenshot and an
# accessibility dump per step under OUT_DIR/steps, and leaves the fixture up
# (the iOS step tears it down). The token for the dashboard goes to
# $RUNNER_TEMP/demo-token, never into OUT_DIR.
set -uo pipefail

if [ "${CI:-}" != "true" ] || [ "${GITHUB_ACTIONS:-}" != "true" ]; then
  echo "record-mac.sh: runs on a CI runner only (it takes over the screen, mouse and keyboard)" >&2
  exit 2
fi

APP="${1:?usage: record-mac.sh APP OUT_DIR}"
OUT_ARG="${2:?usage: record-mac.sh APP OUT_DIR}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="${TEND_DEMO_BIN_DIR:-$REPO/target/debug}"
PORT="${TEND_DEMO_PORT:-7979}"
TMUX_BIN="$(command -v tmux)"
mkdir -p "$OUT_ARG/steps"
OUT="$(cd "$OUT_ARG" && pwd)"
AX="$RUNNER_TEMP/ax"
log() { printf '[%s] %s\n' "$(date +%T)" "$*" | tee -a "$OUT/driver.log" >&2; }

swiftc -O "$HERE/ax.swift" -o "$AX" || { log "ax helper did not build"; exit 1; }

# --- The fixture ------------------------------------------------------------
TOKEN=$(openssl rand -hex 24)
printf '%s' "$TOKEN" > "$RUNNER_TEMP/demo-token"
env -u TMUX -u TMUX_PANE \
  TEND_DEMO_TMUX="$TMUX_BIN" \
  TEND_DEMO_BIN_DIR="$BIN_DIR" \
  TEND_DEMO_DASHBOARD="127.0.0.1:$PORT" \
  TEND_DEMO_DASHBOARD_TOKEN="$TOKEN" \
  TEND_DEMO_PANE_SCRIPTS="$HERE/panes" \
  bash "$REPO/docs/demo-setup.sh" >/dev/null || { log "demo-setup failed"; exit 1; }
eval "$(bash "$REPO/scripts/tend-sandbox.sh" env --name tend-demo)"
SOCKET=$TEND_SOCKET
tend() { env -u TMUX -u TMUX_PANE "$BIN_DIR/tend" "$@"; }
tm() { "$TEND_SANDBOX_TMUX" -u -f "$TEND_SANDBOX_TMUX_CONFIG" -S "$TEND_TMUX_SOCKET" "$@"; }
# An agent's own hook, as the fixture fires them (docs/demo-setup.sh).
hook() { # <session> <agent> <event> <json>
  local pane; pane=$(tm display-message -p -t "$1:0.0" '#{pane_id}')
  TMUX="$TEND_SANDBOX_TMUX_ENV" TMUX_PANE="$pane" "$BIN_DIR/tend" hook "$2" --event "$3" <<<"$4" || log "hook $1 $3 failed"
}

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

# --- The stage --------------------------------------------------------------
screen_id=$(displayplacer list | sed -n 's/^Persistent screen id: //p' | head -1)
displayplacer "id:$screen_id res:1920x1080 scaling:off origin:(0,0) degree:0" || log "display mode unchanged"
defaults write com.apple.dock autohide -bool true
defaults write com.apple.dock autohide-delay -float 1000
killall Dock || true
osascript -e 'tell application "Finder" to close every window' >/dev/null 2>&1 || true
osascript -e 'tell application "System Events" to tell every desktop to set picture to "/System/Library/Desktop Pictures/Solid Colors/Space Gray.png"' >/dev/null 2>&1 \
  || log "wallpaper unchanged"

# The app's own first-run and notification surfaces stay closed: the
# recording is of the workbench, and a permission prompt would cover it.
D=dev.tend.mac
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")
defaults write $D tend.onboarding.completedVersion -string "$version"
for key in tend.notifications.attention tend.notifications.finished tend.notifications.usage tend.notifications.sound; do
  defaults write $D "$key" -bool false
done
defaults write $D tend.appearance -string light
defaults write $D tend.showWorkbenchOnLaunch -bool true
defaults write $D NSQuitAlwaysKeepsWindows -bool false

# The fixture must be the one answering: an absent socket would have the
# app start a tendd of its own.
[ "$SOCKET" = /tmp/tend-demo-sandbox/tendd.sock ] && [ -S "$SOCKET" ] \
  && tend status >/dev/null 2>&1 || { log "the fixture's tendd is not answering at $SOCKET"; exit 1; }
TEND_SOCKET="$SOCKET" "$APP/Contents/MacOS/Tend" > "$OUT/app.log" 2>&1 &
PID=$!
for _ in $(seq 1 60); do
  "$AX" find "$PID" "Home" >/dev/null 2>&1 && break
  sleep 1
done
"$AX" raise "$PID" 40 56 1840 920 || log "window not moved"
osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $PID) to true" || true
# Let the board, the decision queue and the pane previews arrive.
sleep 12
"$AX" tree "$PID" > "$OUT/steps/00-start.ax.txt" 2>&1 || true
screencapture -x "$OUT/steps/00-start.png"

# --- Recording helpers --------------------------------------------------------
STEP=0
step() { STEP=$((STEP + 1)); local n; n=$(printf '%02d-%s' "$STEP" "$1")
  screencapture -x "$OUT/steps/$n.png"; "$AX" tree "$PID" > "$OUT/steps/$n.ax.txt" 2>&1 || true; }
mark() { echo "$1 $(python3 -c "import time;print(round(time.time()-$T0,2))")" >> "$OUT/marks.txt"; log "mark $1"; }
# Centre of the first element labelled TEXT (optionally of ROLE).
centre() { local f; f=$("$AX" find "$PID" "$@") || return 1; set -- $f; echo "$(( $1 + $3 / 2 )) $(( $2 + $4 / 2 ))"; }
glide() { cliclick -e 500 "m:$1,$2"; }
point() { local c; c=$(centre "$@") || { log "not found: $*"; return 1; }; set -- $c; glide "$1" "$2"; }
press() { local c; c=$(centre "$@") || { log "not found: $*"; return 1; }; set -- $c; glide "$1" "$2"; sleep 0.25; cliclick "c:$1,$2"; }
type_slowly() { cliclick -w 45 "t:$1"; }

# --- Take --------------------------------------------------------------------
glide 960 1060
ffmpeg -hide_banner -loglevel error -y -f avfoundation -framerate 30 -capture_cursor 1 -capture_mouse_clicks 0 \
  -i "Capture screen 0" -c:v libx264 -preset ultrafast -crf 16 -pix_fmt yuv420p "$OUT/mac-raw.mp4" < /dev/null &
FF=$!
T0=$(python3 -c 'import time;print(time.time())')
: > "$OUT/marks.txt"
sleep 2

# 1. Home.
mark home
glide 330 330; sleep 1.2
glide 960 380; sleep 1.2
glide 1560 420; sleep 1.2
glide 700 600; sleep 1
step home

# 2. A question: open the agent asking a choice and answer it.
mark question
press "rotate the JWT signing keys" || true
sleep 2.5
step question-open
press "staging, then prod" AXButton || press "staging, then prod" || true
sleep 1.5
step question-answered
# The pane takes the answer and the agent goes back to work
# (scripts/demo-video/panes/auth).
sleep 3
cliclick kp:esc || true
sleep 1
step question-done

# 3. Review: decide a finished result.
mark review
press "Review" AXButton || press "Review" || true
sleep 2.5
step review-open
press "merge" AXButton || true
sleep 1
press "Accept" AXButton || press "Accept" || true
sleep 2
step review-accepted
press "Done" AXButton || cliclick kp:esc || true
sleep 1.5

# 4. Explore: the topology and a live pane.
mark explore
press "Explore" || true
sleep 2.5
step explore
point "Execution topology" || true
sleep 1.5
press "500 from the cod" || press "sdk" || true
sleep 3
step explore-sdk

# 5. Steer: send the agent a prompt.
mark steer
press "Send a prompt" || true
sleep 0.6
type_slowly "retry with the cached spec and open a PR"
sleep 0.8
press "Send" AXButton || cliclick kp:return || true
sleep 4
step steer

# 6. The menu bar.
mark menubar
extra=$("$AX" find-extra "$PID" "Tend" 2>/dev/null | head -1)
[ -n "$extra" ] || extra=$("$AX" extras "$PID" | head -1)
"$AX" extras "$PID" > "$OUT/steps/extras.txt" 2>&1 || true
if [ -n "$extra" ]; then
  set -- $extra
  glide $(( $1 + $3 / 2 )) $(( $2 + $4 / 2 )); sleep 0.3; cliclick c:.
else
  log "no menu bar extra"
fi
sleep 2
step menubar
glide 1600 400; sleep 3
cliclick kp:esc || true
sleep 1
mark end
sleep 1

kill -INT "$FF"; wait "$FF" || true
log "take done: $(ffprobe -v error -show_entries format=duration -of csv=p=0 "$OUT/mac-raw.mp4")s"
kill "$PID" 2>/dev/null || true
exit 0
