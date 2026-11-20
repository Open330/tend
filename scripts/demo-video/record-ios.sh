#!/usr/bin/env bash
# Record the iPhone and iPad part of the product demo video.
#
#   scripts/demo-video/record-ios.sh OUT_DIR
#
# CI only, after record-mac.sh: the iOS app pairs with the demo fixture's
# dashboard (127.0.0.1:$PORT, token in $RUNNER_TEMP/demo-token) and
# DemoTourUITests walks Home while `simctl io recordVideo` records. Each
# device is created for this run and deleted at the end. Writes
# OUT_DIR/{iphone,ipad}-raw.mp4 and OUT_DIR/{iphone,ipad}-marks.txt
# (`<scene> <seconds into that clip>`). Tears the fixture down on exit.
set -uo pipefail

if [ "${CI:-}" != "true" ] || [ "${GITHUB_ACTIONS:-}" != "true" ]; then
  echo "record-ios.sh: runs on a CI runner only" >&2
  exit 2
fi
OUT_ARG="${1:?usage: record-ios.sh OUT_DIR}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
PORT="${TEND_DEMO_PORT:-7979}"
mkdir -p "$OUT_ARG"
OUT="$(cd "$OUT_ARG" && pwd)"
TOKEN=$(cat "$RUNNER_TEMP/demo-token")
DERIVED="$RUNNER_TEMP/ios-derived"
created=()

cleanup() {
  for udid in "${created[@]}"; do
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
    xcrun simctl delete "$udid" >/dev/null 2>&1 || true
  done
  env -u TMUX -u TMUX_PANE TEND_DEMO_TMUX="$(command -v tmux)" bash "$REPO/docs/demo-teardown.sh" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "$REPO/apps/tend-ios"
xcodegen generate >/dev/null
runtime=$(xcrun simctl list runtimes -j | python3 -c 'import json,sys; r=[x for x in json.load(sys.stdin)["runtimes"] if x["isAvailable"] and x["platform"]=="iOS"]; print(r[-1]["identifier"])')
pick() { # <name prefix> → device type id
  xcrun simctl list devicetypes -j | python3 -c "import json,sys; t=[d for d in json.load(sys.stdin)['devicetypes'] if d['name'].startswith('$1')]; e=[d for d in t if d['name']=='$1']; print((e or t)[-1]['identifier'])"
}

xcodebuild -project TendiOS.xcodeproj -scheme Tend -configuration Debug \
  -derivedDataPath "$DERIVED" -destination 'generic/platform=iOS Simulator' \
  CODE_SIGN_IDENTITY=- build-for-testing > "$OUT/ios-build.log" 2>&1 || { tail -40 "$OUT/ios-build.log"; exit 1; }

record() { # <slug> <device type prefix>
  local slug=$1 type udid rec start
  type=$(pick "$2")
  udid=$(xcrun simctl create "tend-demo-$slug" "$type" "$runtime")
  created+=("$udid")
  xcrun simctl boot "$udid"
  xcrun simctl bootstatus "$udid" -b >/dev/null
  # A clean status bar: a fixed time, full signal and battery.
  xcrun simctl status_bar "$udid" override --time "9:41" --dataNetwork wifi --wifiBars 3 \
    --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100 || true
  # Each line simctl prints, stamped: "Recording started" is the clip's 0 s.
  xcrun simctl io "$udid" recordVideo --codec h264 --force "$OUT/$slug-raw.mp4" 2>&1 \
    | python3 -u -c 'import sys,time
for l in sys.stdin: print("%.3f %s" % (time.time(), l.rstrip()), flush=True)' > "$OUT/$slug-rec.log" &
  rec=$!
  for _ in $(seq 1 50); do grep -q "Recording started" "$OUT/$slug-rec.log" && break; sleep 0.2; done
  start=$(awk '/Recording started/ {print $1; exit}' "$OUT/$slug-rec.log")
  TEST_RUNNER_TEND_DEMO_TOUR=1 \
  TEST_RUNNER_TEND_SERVER="http://127.0.0.1:$PORT" \
  TEST_RUNNER_TEND_TOKEN="$TOKEN" \
    xcodebuild -project TendiOS.xcodeproj -scheme Tend -configuration Debug \
      -derivedDataPath "$DERIVED" -destination "id=$udid" \
      -only-testing:TendUITests/DemoTourUITests \
      CODE_SIGN_IDENTITY=- test-without-building > "$OUT/$slug-test.log" 2>&1 || echo "record-ios.sh: $slug tour failed" >&2
  sleep 1
  pkill -INT -f "recordVideo --codec h264 --force $OUT/$slug-raw.mp4" || true
  wait "$rec" || true
  grep -o 'DEMO-MARK [a-z]* [0-9.]*' "$OUT/$slug-test.log" | sort -u -k3 -n \
    | awk -v s="$start" '{printf "%s %.2f\n", $2, $3 - s}' > "$OUT/$slug-marks.txt"
  xcrun simctl shutdown "$udid" || true
}

record iphone "iPhone 17 Pro"
record ipad "iPad Pro 13"
ls -la "$OUT"
