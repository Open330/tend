# Tend for iPhone and iPad

A client of one tend dashboard: it pairs as a device that may only decide,
shows what needs you on every host the dashboard covers, and answers
questions and decides results through the dashboard's own decision routes.
Design and plan: [docs/proposals/IOS_APP.md](../../docs/proposals/IOS_APP.md).
Pairing on the server: [DASHBOARD.md](../../docs/DASHBOARD.md#paired-devices).

| Path | What |
| --- | --- |
| `Sources/` | The app (SwiftUI, iOS 17+, one universal target). |
| `../shared/Sources/` | Decision models shared with Tend for Mac. |
| `../shared/Fixtures/` | Responses recorded from a real tendd, decoded by the unit tests. |
| `Tests/` | Unit tests (origin rules, fixture decoding). |
| `UITests/` | The pair → answer → decide → Decided → sign out flow against a scratch tendd. |
| `Scripts/scratch-tendd.sh` | A sandboxed scratch tendd with the dashboard on `127.0.0.1:7979`, seeded with a question and a result. |

## Build and test

```sh
cd apps/tend-ios
xcodegen generate

# Unit tests (what CI runs).
xcodebuild -project TendiOS.xcodeproj -scheme Tend \
  -destination 'platform=iOS Simulator,name=tend-test-iphone' \
  -only-testing:TendTests CODE_SIGN_IDENTITY=- test

# UI tests against a scratch tendd (built from this checkout).
cargo build -p tendd -p tend-cli
Scripts/scratch-tendd.sh up        # prints server= and token=
TEST_RUNNER_TEND_SERVER=http://127.0.0.1:7979 \
TEST_RUNNER_TEND_TOKEN=$(Scripts/scratch-tendd.sh token) \
xcodebuild -project TendiOS.xcodeproj -scheme Tend \
  -destination 'platform=iOS Simulator,name=tend-test-ipad' \
  CODE_SIGN_IDENTITY=- test
Scripts/scratch-tendd.sh seed      # another question + result for the next run
Scripts/scratch-tendd.sh down
```

`CODE_SIGN_IDENTITY=-` signs to run locally, which the simulator needs for
the Keychain. Without the two `TEST_RUNNER_` variables the UI test skips.

The app never needs the dashboard token. In the simulator, plain `http://`
works for `127.0.0.1` only; anything else must be `https://`.
