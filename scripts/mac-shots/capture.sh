#!/usr/bin/env bash
# Draw every Tend for Mac screen off-screen against the demo fixture fleet,
# light and dark, in English and Korean, for design reviews.
#
#   scripts/mac-shots/capture.sh OUT_DIR
#
# For a CI runner (.github/workflows/mac-shots.yml): it runs the app's test
# host, which a developer's Mac would show. Stands up docs/demo-setup.sh on
# the isolated sandbox, files two results waiting for a decision, and runs
# DesignCaptureTests once per language with the sandbox's socket. Needs
# target/debug/{tend,tendd}, tmux, and the generated Tend.xcodeproj.
set -euo pipefail

OUT="${1:?usage: capture.sh OUT_DIR}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="${TEND_SHOTS_BIN_DIR:-$REPO/target/debug}"
mkdir -p "$OUT"

cleanup() { bash "$REPO/docs/demo-teardown.sh" >/dev/null 2>&1 || true; }
trap cleanup EXIT

env -u TMUX -u TMUX_PANE \
  TEND_DEMO_TMUX="${TEND_DEMO_TMUX:-$(command -v tmux)}" \
  TEND_DEMO_BIN_DIR="$BIN_DIR" \
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

cd "$REPO/apps/tend-macos"
status=0
for lang in en ko; do
  region=US; [ "$lang" = ko ] && region=KR
  TEST_RUNNER_TEND_CAPTURE_SOCKET="$TEND_SOCKET" \
  TEST_RUNNER_TEND_RENDER_DIR="$OUT" \
    xcodebuild \
      -project Tend.xcodeproj -scheme Tend -configuration Debug \
      -derivedDataPath .build/DerivedData -destination 'platform=macOS' \
      -testLanguage "$lang" -testRegion "$region" \
      -only-testing:TendTests/DesignCaptureTests \
      CODE_SIGNING_ALLOWED=NO \
      test > "$OUT/xcodebuild-$lang.log" 2>&1 || status=$?
  grep -E ': error: |Test Case .* failed|timed out waiting|did not render|\*\* TEST ' "$OUT/xcodebuild-$lang.log" | sort -u | head -50 || true
done
ls "$OUT"/*.png | wc -l | xargs echo "screens:"
exit "$status"
