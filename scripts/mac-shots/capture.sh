#!/usr/bin/env bash
# Draw every Tend for Mac screen off-screen against the demo fixture fleet,
# light and dark, in English and Korean, for design reviews.
#
#   scripts/mac-shots/capture.sh OUT_DIR
#
# CI only (.github/workflows/mac-shots.yml): it runs the app's test host, and
# the fixture takes the fixed `tend-demo` sandbox, so it refuses to run
# anywhere but a CI runner. Stands up docs/demo-setup.sh on that sandbox,
# files two results waiting for a decision, and runs DesignCaptureTests once
# per language against the sandbox's socket. Needs target/debug/{tend,tendd},
# tmux, and the generated Tend.xcodeproj.
set -euo pipefail

if [ "${CI:-}" != "true" ] || [ "${GITHUB_ACTIONS:-}" != "true" ]; then
  echo "capture.sh: runs on a CI runner only (it starts the app's test host and the shared tend-demo sandbox)" >&2
  exit 2
fi

OUT_ARG="${1:?usage: capture.sh OUT_DIR}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN_DIR="${TEND_SHOTS_BIN_DIR:-$REPO/target/debug}"
TMUX_BIN="${TEND_DEMO_TMUX:-$(command -v tmux)}"
mkdir -p "$OUT_ARG"
OUT="$(cd "$OUT_ARG" && pwd)"
# The paths scripts/tend-sandbox.sh gives the `tend-demo` sandbox.
FIXTURE_ROOT=/tmp/tend-demo-sandbox
FIXTURE_SOCKET=$FIXTURE_ROOT/tendd.sock

# Teardown runs on every exit, interrupt included, and its failure is the
# run's failure unless something failed first.
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ! env -u TMUX -u TMUX_PANE TEND_DEMO_TMUX="$TMUX_BIN" bash "$REPO/docs/demo-teardown.sh"; then
    echo "capture.sh: teardown failed" >&2
    [ "$status" -eq 0 ] && status=1
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

env -u TMUX -u TMUX_PANE \
  TEND_DEMO_TMUX="$TMUX_BIN" \
  TEND_DEMO_BIN_DIR="$BIN_DIR" \
  bash "$REPO/docs/demo-setup.sh" >/dev/null

# The sandbox's environment, or nothing: a failed export must not leave the
# fixture commands below on an inherited or default socket.
sandbox_env=$(bash "$REPO/scripts/tend-sandbox.sh" env --name tend-demo)
eval "$sandbox_env"
if [[ "${TEND_SOCKET:-}" != "$FIXTURE_SOCKET" || "${TEND_CONFIG:-}" != "$FIXTURE_ROOT"/* || ! -S "$FIXTURE_SOCKET" ]]; then
  echo "capture.sh: the sandbox did not come up at $FIXTURE_SOCKET (TEND_SOCKET=${TEND_SOCKET:-unset})" >&2
  exit 1
fi
tend() { env -u TMUX -u TMUX_PANE "$BIN_DIR/tend" --socket "$FIXTURE_SOCKET" --config "$TEND_CONFIG" "$@"; }

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

# What each language must produce: a missing one fails the run.
screens=(home home-compact home-narrow home-compact-narrow home-hosts home-hosts-narrow home-review home-start-work home-material explore-material explore inbox ask detail-work detail-agent detail-host menubar onboarding
  onboarding-checklist onboarding-firstAgent onboarding-notifications onboarding-workbench onboarding-flow onboarding-done
  work-templates automation-editor-new automation-editor automation-test whats-new home-unconnected
  settings-general settings-notifications settings-menuBar settings-accounts settings-ask settings-agents
  settings-automations settings-fleet settings-advanced)

cd "$REPO/apps/tend-macos"
status=0
for lang in en ko; do
  region=US; [ "$lang" = ko ] && region=KR
  rm -f "$OUT"/*-"$lang".png
  TEST_RUNNER_TEND_CAPTURE_SOCKET="$FIXTURE_SOCKET" \
  TEST_RUNNER_TEND_RENDER_DIR="$OUT" \
    xcodebuild \
      -project Tend.xcodeproj -scheme Tend -configuration Debug \
      -derivedDataPath .build/DerivedData -destination 'platform=macOS' \
      -testLanguage "$lang" -testRegion "$region" \
      -only-testing:TendTests/DesignCaptureTests \
      CODE_SIGNING_ALLOWED=NO \
      test > "$OUT/xcodebuild-$lang.log" 2>&1 || status=$?
  grep -E ': error: |Test Case .* failed|timed out waiting|did not render|must be the running fixture|\*\* TEST ' "$OUT/xcodebuild-$lang.log" | sort -u | head -50 || true
  for screen in "${screens[@]}"; do
    for scheme in light dark; do
      if [ ! -s "$OUT/$screen-$scheme-$lang.png" ]; then
        echo "capture.sh: missing $screen-$scheme-$lang.png" >&2
        status=1
      fi
    done
  done
done
echo "screens: $(find "$OUT" -name '*.png' | wc -l | tr -d ' ')"
exit "$status"
