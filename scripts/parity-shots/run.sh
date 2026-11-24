#!/usr/bin/env bash
# Screens of Tend for Mac, the web dashboard and the iOS app, all drawn
# against ONE demo fixture, for the cross-platform parity audit.
#
#   scripts/parity-shots/run.sh OUT_DIR
#
# CI only (.github/workflows/parity-shots.yml, a GitHub-hosted runner): it
# stands up docs/demo-setup.sh on the `tend-demo` sandbox with its dashboard
# on 127.0.0.1:$PORT (as scripts/demo-video/record-mac.sh does), files two
# results waiting for a decision, then
#   1. Mac  DesignCaptureTests off-screen, English and Korean, light and dark
#   2. web  scripts/parity-shots/web.mjs, headless Chromium (Playwright)
#   3. iOS  ParityShotsUITests on an iPhone and an iPad simulator created
#           for the run and deleted at the end
# Nothing answers, decides or starts anything: every surface shows the
# fixture's open state. Writes OUT_DIR/<surface>-<screen>-<variant>.png, a
# .txt of each screen's visible text beside it, the fixture's /api/home
# before and after, and manifest.txt. Needs target/debug/{tend,tendd},
# tmux, xcodegen, the generated Tend.xcodeproj (libghostty built), node and
# TEND_PLAYWRIGHT_PACKAGE (a Playwright with its Chromium installed).
set -uo pipefail

if [ "${CI:-}" != "true" ] || [ "${GITHUB_ACTIONS:-}" != "true" ] \
   || [ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" ] || [ ! -d "${RUNNER_TEMP:-}" ]; then
  echo "run.sh: runs on a GitHub-hosted runner only (it starts the app's test host, simulators and the shared tend-demo sandbox)" >&2
  exit 2
fi

OUT_ARG="${1:?usage: run.sh OUT_DIR}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="${TEND_DEMO_BIN_DIR:-$REPO/target/debug}"
PORT="${TEND_DEMO_PORT:-7979}"
TMUX_BIN="$(command -v tmux)"
mkdir -p "$OUT_ARG"
OUT="$(cd "$OUT_ARG" && pwd)"
LOGS="$RUNNER_TEMP/parity-logs"
mkdir -p "$LOGS"
log() { printf '[%s] %s\n' "$(date +%T)" "$*" | tee -a "$LOGS/driver.log" >&2; }
status=0
fail() { log "FAILED: $*"; status=1; }

TOKEN=$(openssl rand -hex 24)
created=()
helper=
cleanup() {
  local code=$?
  trap - EXIT INT TERM
  [ -n "$helper" ] && kill "$helper" 2>/dev/null
  for udid in "${created[@]}"; do
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
    xcrun simctl delete "$udid" >/dev/null 2>&1 || true
  done
  env -u TMUX -u TMUX_PANE TEND_DEMO_TMUX="$TMUX_BIN" bash "$REPO/docs/demo-teardown.sh" >/dev/null 2>&1 \
    || log "teardown failed"
  exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# --- The fixture ------------------------------------------------------------
log "fixture"
env -u TMUX -u TMUX_PANE \
  TEND_DEMO_TMUX="$TMUX_BIN" \
  TEND_DEMO_BIN_DIR="$BIN_DIR" \
  TEND_DEMO_DASHBOARD="127.0.0.1:$PORT" \
  TEND_DEMO_DASHBOARD_TOKEN="$TOKEN" \
  bash "$REPO/docs/demo-setup.sh" > "$LOGS/demo-setup.log" 2>&1 || { log "demo-setup failed"; tail -30 "$LOGS/demo-setup.log"; exit 1; }
# The sandbox's environment, or nothing (as mac-shots/capture.sh).
sandbox_env=$(bash "$REPO/scripts/tend-sandbox.sh" env --name tend-demo) || { log "no sandbox environment"; exit 1; }
eval "$sandbox_env"
FIXTURE_ROOT=/tmp/tend-demo-sandbox
SOCKET=$FIXTURE_ROOT/tendd.sock
if [[ "${TEND_SOCKET:-}" != "$SOCKET" || "${TEND_CONFIG:-}" != "$FIXTURE_ROOT"/* || ! -S "$SOCKET" ]]; then
  log "the sandbox did not come up at $SOCKET (TEND_SOCKET=${TEND_SOCKET:-unset})"
  exit 1
fi
tend() { env -u TMUX -u TMUX_PANE "$BIN_DIR/tend" --socket "$SOCKET" --config "$TEND_CONFIG" "$@"; }
tend status >/dev/null 2>&1 || { log "the fixture's tendd is not answering at $SOCKET"; exit 1; }

# Two finished results for Your turn, as mac-shots and the demo video file them.
tend review request --session s-billing --as agent --label billing \
  --summary "Proration now settles a plan that changes twice in one period" \
  --decision-needed "Merge the proration fix?" \
  --option "merge" --option "wait for finance" \
  --recommend "merge: the two-change case has a test now" \
  --evidence "test-pass:cargo test -p billing proration" >/dev/null || fail "review request (billing)"
tend review request --session s-data --as agent --label data \
  --summary "Analytics rollups backfilled for last week" \
  --decision-needed "Publish the backfilled rollups?" \
  --option "publish" --option "hold" \
  --recommend "publish: 14 models ran with 0 errors" >/dev/null || fail "review request (data)"

waited=0
until curl -fsS -o /dev/null -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/home"; do
  waited=$((waited + 1)); [ "$waited" -lt 100 ] || { log "the dashboard never came up"; exit 1; }
  sleep 0.3
done
snapshot() { # <name>: what the fixture's board holds, for comparing counts
  curl -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/home" > "$OUT/fixture-$1.json" 2>/dev/null \
    || log "no /api/home snapshot ($1)"
  python3 - "$OUT/fixture-$1.json" > "$OUT/fixture-$1.txt" 2>/dev/null <<'PY' || true
import json, sys
home = json.load(open(sys.argv[1]))
def walk(value, path=""):
    if isinstance(value, dict):
        for key, item in value.items():
            if key in ("count", "total") and isinstance(item, int):
                print(f"{path}.{key} = {item}")
            walk(item, f"{path}.{key}")
    elif isinstance(value, list):
        if path.endswith(("units", "items", "decisions", "groups", "hosts", "agents")):
            print(f"{path}[] = {len(value)}")
        for index, item in enumerate(value):
            walk(item, f"{path}[{index}]" if not isinstance(item, (dict, list)) else f"{path}[{index}]")
walk(home)
PY
}
snapshot before
# Let the fixture's agents, panes and decisions settle.
sleep 5

# --- 1. Tend for Mac --------------------------------------------------------
log "mac"
MAC_RAW="$RUNNER_TEMP/mac-raw"
rm -rf "$MAC_RAW"; mkdir -p "$MAC_RAW"
(
  cd "$REPO/apps/tend-macos" || exit 1
  for lang in en ko; do
    region=US; [ "$lang" = ko ] && region=KR
    TEST_RUNNER_TEND_CAPTURE_SOCKET="$SOCKET" \
    TEST_RUNNER_TEND_RENDER_DIR="$MAC_RAW" \
    TEST_RUNNER_TEND_RENDER_TEXT=1 \
      xcodebuild \
        -project Tend.xcodeproj -scheme Tend -configuration Debug \
        -derivedDataPath .build/DerivedData -destination 'platform=macOS' \
        -testLanguage "$lang" -testRegion "$region" \
        -only-testing:TendTests/DesignCaptureTests \
        CODE_SIGNING_ALLOWED=NO \
        test > "$LOGS/mac-$lang.log" 2>&1 || echo "mac $lang: xcodebuild exited $?" >&2
    grep -E ': error: |Test Case .* failed|timed out waiting|did not render|must be the running fixture|\*\* TEST ' "$LOGS/mac-$lang.log" | sort -u | head -30 || true
  done
)
for file in "$MAC_RAW"/*; do
  [ -e "$file" ] || continue
  mv "$file" "$OUT/mac-$(basename "$file")"
done
# Where the off-screen accessibility tree came back empty, the screen's
# words as Vision reads them (scripts/demo-video/ocr.swift), marked as such.
OCR_DIR="$RUNNER_TEMP/mac-ocr"
rm -rf "$OCR_DIR"; mkdir -p "$OCR_DIR"
for png in "$OUT"/mac-*.png; do
  [ -e "$png" ] || continue
  txt="${png%.png}.txt"
  if [ ! -s "$txt" ] || [ "$(grep -c '[^[:space:]]' "$txt")" -eq 0 ]; then ln -s "$png" "$OCR_DIR/"; fi
done
if [ -n "$(ls -A "$OCR_DIR")" ] && swiftc -O "$REPO/scripts/demo-video/ocr.swift" -o "$RUNNER_TEMP/ocr" 2>> "$LOGS/ocr.log"; then
  "$RUNNER_TEMP/ocr" "$OCR_DIR" > "$RUNNER_TEMP/mac-ocr.tsv" 2>> "$LOGS/ocr.log" || log "mac: OCR failed"
  for link in "$OCR_DIR"/*.png; do
    name=$(basename "$link")
    { echo "# OCR (Vision): the accessibility tree was empty off-screen"
      awk -F '\t' -v f="$name" '$1 == f { sub(/^[^\t]*\t/, ""); print }' "$RUNNER_TEMP/mac-ocr.tsv"
    } > "$OUT/${name%.png}.txt"
  done
  log "mac: OCR text for $(ls "$OCR_DIR" | wc -l | tr -d ' ') screens"
fi
mac_count=$(find "$OUT" -name 'mac-*.png' | wc -l | tr -d ' ')
log "mac: $mac_count screens"
[ "$mac_count" -gt 0 ] || fail "no Mac screens"

# --- 2. The web dashboard ---------------------------------------------------
log "web"
TEND_PARITY_URL="http://127.0.0.1:$PORT" TEND_PARITY_TOKEN="$TOKEN" TEND_PARITY_OUT="$OUT" \
  node "$HERE/web.mjs" > "$LOGS/web.log" 2>&1 || fail "web.mjs exited non-zero"
tail -5 "$LOGS/web.log" >&2
log "web: $(find "$OUT" -name 'web-*.png' | wc -l | tr -d ' ') screens"

# --- 3. iPhone and iPad -----------------------------------------------------
log "ios"
DERIVED="$RUNNER_TEMP/ios-derived"
(
  cd "$REPO/apps/tend-ios" && xcodegen generate >/dev/null \
  && xcodebuild -project TendiOS.xcodeproj -scheme Tend -configuration Debug \
    -derivedDataPath "$DERIVED" -destination 'generic/platform=iOS Simulator' \
    CODE_SIGN_IDENTITY=- build-for-testing > "$LOGS/ios-build.log" 2>&1
) || { fail "the iOS build failed"; grep -E ': error: ' "$LOGS/ios-build.log" | head -20; tail -20 "$LOGS/ios-build.log"; }

runtime=$(xcrun simctl list runtimes -j | python3 -c 'import json,sys; r=[x for x in json.load(sys.stdin)["runtimes"] if x["isAvailable"] and x["platform"]=="iOS"]; print(r[-1]["identifier"])')
pick() { # <name prefix> -> device type id
  xcrun simctl list devicetypes -j | python3 -c "import json,sys; t=[d for d in json.load(sys.stdin)['devicetypes'] if d['name'].startswith('$1')]; e=[d for d in t if d['name']=='$1']; print((e or t)[-1]['identifier'])"
}

# Takes each screen the UI test asks for with `simctl io screenshot`.
shoot_helper() { # <slug> <udid>
  local slug=$1 udid=$2 request="$OUT/request-$1" name
  while :; do
    if [ -f "$request" ]; then
      name=$(cat "$request"); rm -f "$request"
      case "$name" in ios-*) ;; *) continue ;; esac
      xcrun simctl io "$udid" screenshot --type=png "$OUT/$name.png" >/dev/null 2>&1 || log "simctl screenshot $name failed"
      : > "$OUT/done-$name"
    fi
    sleep 0.2
  done
}

ios_run() { # <slug> <device type prefix>
  local slug=$1 type udid bundle="$RUNNER_TEMP/ios-$1.xcresult"
  type=$(pick "$2") || { fail "no $2 device type"; return; }
  udid=$(xcrun simctl create "tend-parity-$slug" "$type" "$runtime") || { fail "no $slug simulator"; return; }
  created+=("$udid")
  xcrun simctl boot "$udid"
  xcrun simctl bootstatus "$udid" -b >/dev/null
  xcrun simctl status_bar "$udid" override --time "9:41" --dataNetwork wifi --wifiBars 3 \
    --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100 || true
  shoot_helper "$slug" "$udid" &
  helper=$!
  rm -rf "$bundle"
  TEST_RUNNER_TEND_PARITY=1 \
  TEST_RUNNER_TEND_SERVER="http://127.0.0.1:$PORT" \
  TEST_RUNNER_TEND_TOKEN="$TOKEN" \
  TEST_RUNNER_TEND_PARITY_DIR="$OUT" \
    xcodebuild -project "$REPO/apps/tend-ios/TendiOS.xcodeproj" -scheme Tend -configuration Debug \
      -derivedDataPath "$DERIVED" -destination "id=$udid" -resultBundlePath "$bundle" \
      -only-testing:TendUITests/ParityShotsUITests \
      CODE_SIGN_IDENTITY=- test-without-building > "$LOGS/ios-$slug.log" 2>&1 \
    || { log "ios $slug: the UI test reported failures"; grep -E ': error: |failed' "$LOGS/ios-$slug.log" | head -10 >&2; }
  kill "$helper" 2>/dev/null; wait "$helper" 2>/dev/null; helper=
  rm -f "$OUT"/request-"$slug" "$OUT"/done-*
  # The attachments, for any screen the helper did not take.
  local exported="$RUNNER_TEMP/ios-$slug-attachments"
  rm -rf "$exported"; mkdir -p "$exported"
  if xcrun xcresulttool export attachments --path "$bundle" --output-path "$exported" > "$LOGS/ios-$slug-export.log" 2>&1; then
    python3 - "$exported" "$OUT" <<'PY' || log "ios $slug: attachments not mapped"
import json, os, re, shutil, sys
src, out = sys.argv[1], sys.argv[2]
manifest = json.load(open(os.path.join(src, "manifest.json")))
for test in manifest if isinstance(manifest, list) else [manifest]:
    for item in test.get("attachments", []):
        name = item.get("suggestedHumanReadableName") or ""
        stem, ext = os.path.splitext(name)
        stem = re.sub(r"_\d+_[0-9A-Fa-f-]{36}$", "", stem)
        if not stem.startswith("ios-"):
            continue
        if stem.endswith(".txt"):
            stem, ext = stem[:-4], ".txt"
        target = os.path.join(out, stem + (ext or ".png"))
        if not os.path.exists(target):
            shutil.copyfile(os.path.join(src, item["exportedFileName"]), target)
PY
  else
    log "ios $slug: xcresulttool export failed"
  fi
  xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  log "ios $slug: $(find "$OUT" -name "ios-*-$slug.png" | wc -l | tr -d ' ') screens"
  [ -n "$(find "$OUT" -name "ios-*-$slug.png" -print -quit)" ] || fail "no $slug screens"
}

if [ -d "$DERIVED/Build/Products" ]; then
  ios_run iphone "iPhone 17 Pro"
  ios_run ipad "iPad Pro 13"
fi

snapshot after

# --- What came back ---------------------------------------------------------
# The dashboard's token never leaves the runner.
leaks=$(grep -rlF "$TOKEN" "$OUT" 2>/dev/null || true)
if [ -n "$leaks" ]; then
  log "the dashboard token appeared in: $leaks (removed)"
  printf '%s\n' "$leaks" | while read -r file; do rm -f "$file"; done
  fail "token in the output"
fi
grep -rlF "$TOKEN" "$LOGS" 2>/dev/null | while read -r file; do sed -i '' "s/$TOKEN/<token>/g" "$file"; done
{
  echo "# Tend parity shots: one demo fixture (docs/demo-setup.sh, tend-demo sandbox), three surfaces"
  echo "# commit ${GITHUB_SHA:-unknown}  run ${GITHUB_RUN_ID:-unknown}  $(date -u +%FT%TZ)"
  echo "# <surface>-<screen>-<variant>.png, with <same>.txt = the screen's visible text"
  echo "# mac: <screen>-<light|dark>-<en|ko>; web: <screen>-<desktop|phone>-<light|dark>[-ko]; ios: <screen>-<iphone|ipad>"
  echo "# fixture-before/after.{json,txt}: the dashboard's /api/home when the capture began and ended"
  for surface in mac web ios; do
    echo
    echo "## $surface: $(find "$OUT" -maxdepth 1 -name "$surface-*.png" | wc -l | tr -d ' ') screens"
    (cd "$OUT" && find . -maxdepth 1 -name "$surface-*.png" | sed 's|^\./||' | sort | while read -r png; do
      size=$(sips -g pixelWidth -g pixelHeight "$png" 2>/dev/null | awk '/pixelWidth/{w=$2} /pixelHeight/{h=$2} END{print w"x"h}')
      text=-; [ -f "${png%.png}.txt" ] && text=txt
      echo "$png  $size  $text"
    done)
  done
  echo
  echo "## not captured"
  cat "$OUT"/web-missing.txt "$OUT"/ios-missing-*.txt 2>/dev/null || true
} > "$OUT/manifest.txt"
cp "$LOGS"/driver.log "$OUT/driver.log" 2>/dev/null || true
log "done: $(find "$OUT" -name '*.png' | wc -l | tr -d ' ') screens (status $status)"
exit "$status"
