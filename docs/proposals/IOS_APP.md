# Tend on iPhone and iPad

Status: proposal (2026-11-15), wave 5 `ios-app` stream. Phase 1 is being
built alongside this document; see [Plan](#plan).

## Owner decisions

Each item names a recommendation and the default this stream proceeds on
until the owner decides. Nothing here blocks phase 1, which runs only in
the simulator and against a scratch daemon.

| # | Decision | Recommendation | Default meanwhile |
| --- | --- | --- | --- |
| 1 | Apple Developer Program team to sign the app, run TestFlight and hold the push key (USD 99/year if the owner's team is not already enrolled). | Use the team that already signs Tend for Mac. | Simulator builds only, `CODE_SIGNING_ALLOWED=NO`; no device installs. |
| 2 | Bundle identifier. It cannot be changed once registered in App Store Connect. | `dev.tend.ios`, next to the Mac app's `dev.tend.mac`. | `dev.tend.ios` in `project.yml`, not registered anywhere. |
| 3 | Distribution route. | TestFlight internal testing only (owner's devices), no public App Store listing until push (phase 2) has been used for a few weeks. | None. |
| 4 | Who holds the APNs key (phase 2). | The owner's `.p8` key lives on the host that serves the dashboard (jiun-mini), mode 0600, named by path in config; no relay service. See [Push](#push-phase-2). | Push not built. |
| 5 | What a notification shows on the lock screen (phase 2). | Nothing from the agent: "Tend: 2 need you" and the host alias. The question is fetched after unlock. | Push not built. |
| 6 | Keep the iOS CI job phase 1 adds (build + unit tests on a GitHub macOS runner, only when `apps/**` changes; free on a public repository, a few minutes per run). | Keep it: the shared models are edited from the Mac side, and only a full iOS build catches what that breaks. | Added with the iOS app PR; removing it is deleting one job. |

## What the owner uses today

Measured on 2026-11-15, read-only:

- The dashboard runs on jiun-mini (`127.0.0.1:7878`), published as
  `https://tend.jiun.dev` through a Cloudflare tunnel, with OIDC operator
  sign-in (`api.jiun.dev`, `required_group = "operator"`) and the dashboard
  token as the second credential. Fleet (`[fleet] enabled = true`) adds
  june-mba, jiun-mbp and rtzr in control mode, so `/needs` already covers four hosts.
- From the phone the owner opens `/needs` (questions, waiting agents and
  results with their decision context; **Decided** tab). Answering and
  deciding work there.
- **Nothing tells the phone that something needs a decision.** jiun-mini's
  config has no `[sinks]` section, so no webhook or push leaves the host.
  The phone is pull-only: the owner has to remember to look.
- **The phone signs in again every 8 hours.** Operator sessions are in-memory
  cookies with `SESSION_TTL = 8 h` (`crates/tend/src/dashboard/operator/mod.rs`);
  a daemon restart ends them too.
- The page is not an installable web app: `GET /manifest.webmanifest` is 404,
  `needs.html` has no manifest link, no `apple-mobile-web-app-*` tags and no
  service worker.

## What the Mac app does (from the code)

Tend for Mac is 119 Swift files, about 59,000 lines (`apps/tend-macos/Sources`).
It talks only to the local `tendd` over the uid-scoped unix socket
(newline-framed JSON, `protocol: 6`, `hello` capability handshake,
`TendIPC.swift`); it has **no HTTP client**. Other hosts are reached through
the local daemon's Fleet (`fleet_snapshot`, `fleet_command`).

| Area | What it does | How | On iPhone/iPad |
| --- | --- | --- | --- |
| Home: needs you | Decision cards in "Your turn": question, options, recommendation, evidence, tend's own verification; answer, decline, Looks good, Send back, Withdraw (`FleetHomeView.swift`, `DecisionQueue.swift`, `FleetDecisionViews.swift`) | IPC `decision_queue`, `collaboration_reply`, `review_dispose`, `review_record` | **Yes, phase 1**, through the dashboard's `/api/decisions` routes, which serve the same queue per host. |
| Decided history | Past dispositions and answers (`FleetDecisionHistoryView.swift`) | IPC `decision_history` | **Yes, phase 1** (`/api/decisions/history`). |
| Home: in progress / finished lanes, backlog | Work units by lane, inspector, reply field (`FleetHome.swift`, `FleetBacklog.swift`) | IPC `fleet_snapshot`, `work_backlog` | Phase 3, read-only first (`/api/works`, `/api/fleet`). |
| Waiting agents and TUI choice dialogs | Answer a pane's prompt by keys or text (`FleetChoice*.swift`) | IPC `fleet_command` `capture`/`send_keys`/`send_prompt` | Listed in phase 1 (they are in the queue), answered on a computer. Typing into panes from a phone is a separate, wider credential (see [Sign-in](#sign-in-device-pairing)). |
| Hosts | Online/total, host badges, host detail | IPC `fleet_snapshot` | Phase 1 shows each host's section and whether it could be read; detail in phase 3. |
| Agents, panes, live terminal | Explore tree, pane capture, Ghostty terminal (`WatchViews.swift`, `TerminalPaneModel.swift`) | IPC, GhosttyKit (macOS package) | Read-only pane output in phase 3; no terminal. |
| Start Work, pipelines, dispatch, new agent | `WorkStartView`, `PipelineEditorView`, `NewAgentView` | IPC `work_up`, `agent_start`, … | Phase 3 at most ("start this Work"), not the editors. |
| Operator inbox, collaboration history | `OperatorInboxViews.swift` | IPC `collaboration_*` | The questions are in phase 1's queue; the history view is not planned. |
| Ask, Changes/git, Files, Automation rules, Settings (9 tabs), menu bar, global hotkey, onboarding, account module, session snapshots, daemon lifecycle | | IPC, local `git`/`aas`/`launchctl` processes, AppKit | **No.** They configure or inspect a computer; an iPhone cannot run `tendd`, tmux, git or the agents. |
| Notifications | Local notifications for agents needing attention (`AgentAttentionCenter.swift`) | UNUserNotificationCenter fed by IPC | Phase 2 as push; the rules (`TendNotificationRules`) are portable. |

## What a native app adds over the web page

Candidates from the brief, judged against the two measured gaps (no
notification, 8-hour sign-in):

| Candidate | Native | Installed web app (PWA) | Verdict |
| --- | --- | --- | --- |
| Notify when something needs a decision | APNs. Needs an Apple team key and a sender (below). | Web Push works for home-screen web apps on iOS/iPadOS 16.4+, with VAPID keys the daemon can generate itself, no Apple account. | Both can. This is the main value either way. |
| Act from the notification (answer, Looks good, Send back) | Notification actions with text input and `.authenticationRequired` (Face ID / passcode before the decision is sent). | iOS shows web notifications without action buttons; a tap opens the page. | **Native only.** |
| Stay signed in | Per-device credential in the Keychain (this-device-only, not in backups). | Cookie session; would need a longer-lived session design. | Native is simpler to make safe; the server piece (device credentials) is the same work either way. |
| Widgets, Lock Screen count, Live Activity | WidgetKit, ActivityKit. | No. | Native only. Phase 3. |
| Shortcuts / Siri ("what needs me") | App Intents. | No. | Native only. Phase 3. |
| Share sheet, offline queue | Possible. | Possible (service worker), awkward. | Low value: deciding offline is wrong by design (the decision must reach tendd, which may have moved on). Not planned. |
| Better pane output | Native text view, but the same data. | Already there for operators. | Not a reason. |

Honest answer: **a native app's phase 1, on its own, is not much better than
`/needs` in Safari.** What it gives on day one is a sign-in that survives
(device pairing, below) and the Mac app's language and order on the phone.
The owner switches to it for good with phase 2, when a decision arrives as a
notification that can be answered from the lock screen. Phase 1 is still
the right first slice, because phase 2 needs exactly its pieces: a device
identity on the host (what a push registration hangs off), a credential
that cannot do more than decide, and the decision views the notification
opens.

A PWA (manifest + service worker + Web Push) is a cheap complement for
people without an Apple developer team, and it reuses the device
credential design. It belongs to the web dashboard and is not in this
stream; it is listed as a follow-up.

## Transport

The app is a client of **one dashboard origin** (the host that serves the
dashboard, e.g. `https://tend.jiun.dev`). It never opens SSH or a new
listener: other hosts' items come through that daemon's Fleet, exactly as
`/needs` gets them (`GET /api/decisions` returns a section per host, and a
decision on another control-mode host is routed by `host`).

- HTTPS only, with system trust (App Transport Security on). Plain `http`
  is accepted only for a loopback host (`127.0.0.1`, `localhost`, `[::1]`),
  the same rule the daemon applies to `public_url`; that is how the
  simulator reaches a scratch daemon. The entered address must be a bare
  origin: no user info, path, query or fragment.
- **Bound to that origin.** The app follows no redirect (a redirect is an
  error, for pairing and for every authenticated call), and the credential
  is stored under the canonical origin; a different address needs a new
  pairing.
- No certificate pinning: the owner's origin is served by Cloudflare edges
  whose certificates rotate. That means Cloudflare, which terminates TLS
  for `tend.jiun.dev`, sees the device token like it already sees the
  dashboard token and session cookies today.
- The app sends `Authorization: Bearer <device token>` and no cookies (an
  ephemeral `URLSession` without a cookie store), so the dashboard's cookie
  CSRF rules do not apply to it and no cookie can be lifted from it.
- Evidence links (a PR, a commit page) open in Safari, which never gets the
  token.
- Polling, not SSE, in phase 1: the queue is re-read when the app becomes
  active, on pull-to-refresh and every 20 s while it is in the foreground.
  `/api/decisions` is one request that already fans out to Fleet hosts with
  a 6 s timeout per host on the server side.

## Sign-in: device pairing

The phone gets **its own credential, which can only decide**. Pairing is
shaped like the OAuth device authorization grant (RFC 8628), with tend's own
endpoints and responses: the phone asks, an operator approves, and no secret
passes through a link, the clipboard or another app (the device code goes
only from the phone to the daemon).

```text
 iPhone                                          Mac browser, signed in as operator
 ──────                                          ──────────────────────────────────
 enter https://tend.jiun.dev
 POST /auth/devices/request {name, platform}
 ◄── device_code (kept), user_code BCDF-GHJK (shown)
                                                 access ▸ Devices ▸ type BCDF-GHJK
                                                 POST /api/devices/approve ──► "Jiun's iPhone"
 POST /auth/devices/token {device_code}  (every 2 s)
 ◄── 202 pending … 201 {device, token}  (once)
 token ► Keychain (this device only)
                                                 access ▸ Devices: name, approved by,
                                                 last used, Remove, Remove all
```

- **Request.** Unauthenticated, small body. The device code and the token
  are 244 random bits (two UUIDv4s), kept only as SHA-256. The user code is
  8 characters from a 28-letter alphabet without vowels or look-alikes
  (about 38 bits), and only a signed-in operator can submit it, so it is not
  a guessing target. A request lives 10 minutes; at most 32 are pending,
  then 429. There is no global backoff: a wrong device code costs one hash
  lookup and slows nobody else down, and a flood of requests can delay a
  pairing (429) but never yields a credential. Pairing is rare; that is
  accepted.
- **Approve.** `POST /api/devices/approve {user_code}` with the dashboard
  token or an operator session (with the existing CSRF proof). The answer
  names the device after the fact; a device-supplied name proves nothing.
  The remaining risk is RFC 8628's remote phishing (section 5.4): someone
  shows the operator *their* code. The dialog says to approve only a code
  shown on a device you are pairing yourself, right now.
- **Token.** Issued once, atomically under the registry lock (two racing
  polls get one token). The daemon stores the token's SHA-256 with the
  device id, the name the app sent (bounded, control characters removed),
  who approved it (OIDC issuer/subject/email, or `token`), and the times.
- **Scope.** A device token adds access to exactly `GET /api/decisions`,
  `GET /api/decisions/history`, `POST /api/decisions/requests/{id}`,
  `POST /api/decisions/reviews/{id}`, and its own `GET /api/device` /
  `POST /api/device/forget`. It adds nothing anywhere else: no pane output
  or input, Ctrl-C, Fleet commands, Work control, shares, operator, viewer or
  device management. (What a deployment serves anonymously, e.g. reads in
  `public_read` mode, `/auth/session` and static assets, stays as it is.)
  Deciding is still real authority: the device reads every covered host's
  questions and evidence, and an answer can tell an agent to go ahead. The
  app says so on the pairing screen.
- **Lifetime and revocation.** Devices are **their own principals**, not
  delegated sessions of the approving operator: removing that operator, or
  rotating the dashboard token, does not unpair a device. A device stops
  working after 30 days unused or 90 days after pairing, whichever comes
  first, and at once when removed (**Remove**, **Remove all**, or the app's
  Sign out). At most 16. Stored in
  `$XDG_DATA_HOME/tend/dashboard-devices/devices.sqlite3` (0600 in a 0700
  directory, exclusive lock, own application id), created at the first
  pairing.
- **Audit.** A device decision is recorded as `app:<name>#<id prefix>`, so
  two phones with one name stay apart. Its `dashboard-decisions.jsonl` line
  names the device id and who approved it; requests approved, tokens issued
  and removals are written there too, never a code or a token. A decision on
  another Fleet host carries only the label to that host; the full
  attribution is in the dashboard host's audit file.
- **On the phone.** The token is written to the Keychain with
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` (no iCloud Keychain, not
  restored to another phone, unreadable while locked) and never to
  `UserDefaults` or logs. Phase 2's notification actions may need
  after-first-unlock access; that is decided with push, not now.
- **Capability.** `/api/access` reports `capabilities.device_pairing`; an
  older daemon answers `/auth/devices/request` 404 and the app says the host
  needs a newer tend. No IPC change, no protocol bump.

Rejected:

- **The dashboard token on the phone.** It is full operator authority,
  including `POST /api/panes/{pane}/prompt`, which on a shell pane is a
  remote shell. It cannot be revoked for one device without rotating it
  everywhere, and the audit cannot tell the phone from the CLI.
- **An operator-minted code carried to the phone** (a `tend://pair?code=…`
  link, copy, AirDrop). This was the first draft. The code is the whole
  credential in transit: any app that registers the same URL scheme, or
  anyone who sees the clipboard, can claim it first, and the operator cannot
  tell whose phone won.
- **OIDC inside the app, reusing the cookie** (`ASWebAuthenticationSession`).
  The `HttpOnly` cookie lands in a browser jar the app cannot read, lasts 8 h,
  and is full operator authority. Making the daemon an OAuth authorization
  server for native clients (RFC 8252, code + PKCE) would be a sound way to
  *mint the same device credential* through a sign-in; it is a candidate for
  later, not needed to pair the owner's phone.

## Push (phase 2)

APNs accepts a provider token signed with a key from the Apple developer
team that owns the app's bundle identifier. Whoever holds that key can push
to the app's users, so it is a secret of whoever publishes the build; tend
ships none.

- **Chosen:** the host that serves the dashboard sends directly to
  `api.push.apple.com` with the publishing team's key, named by path in
  `[dashboard.push]` (`apns_key_path`, `key_id`, `team_id`, `topic`), file
  mode checked like other secrets. For the owner that is jiun-mini holding
  the owner's key. Use a key restricted to this app's topic and the
  environment in use where App Store Connect offers that, never a team-wide
  key shared with other apps; rotate by adding a new key, switching the
  config, then revoking the old one. Someone who builds the app themselves
  uses their own team's key the same way.
- This does not serve other people's daemons with the owner's build: a
  TestFlight or store build signed by the owner can only be pushed with the
  owner's key, which no one else's host should hold.
- **Rejected for now:** a relay run by the app's publisher, so that people
  installing a store build need no key. It is a hosted service with an
  operator, uptime and a privacy policy; reconsider only with a public App
  Store build.
- The device registers its APNs token with `POST /api/device/push` (device
  credential). The payload carries no agent text: count, host alias, item id.
  Notification actions (Looks good / Send back / Answer) use
  `.authenticationRequired` and post to the same decision routes.
- Simulator verification uses `xcrun simctl push` with the same payload, so
  phase 2 is testable without APNs; delivery from the host needs decision 1.

## Technology

**SwiftUI, one universal target (iPhone and iPad), iOS 17+**, in
`apps/tend-ios/`, generated by XcodeGen like the Mac app. iPad uses
`NavigationSplitView` with the Mac app's sidebar order (Needs you, Decided,
then hosts); iPhone gets the same as a stack with a tab bar.

Rejected:

- **WKWebView shell around `/needs`.** One UI for web and phone is a real
  maintenance win, but the cookie sign-in and CSRF `Origin` rules fight a
  web view, the shell still needs native code for the credential and for
  every notification action, and App Review treats thin web wrappers poorly
  (guideline 4.2). It also leaves the Mac app's words and order behind.
- **Mac Catalyst or a multiplatform port of Tend for Mac.** 40 of its files
  import AppKit; its transport is a unix socket to a local daemon; the
  terminal is a macOS-only package. Little of it is reachable from a phone.
- **React Native / Flutter.** A third toolchain sharing code with neither the
  Mac app nor the dashboard.
- **PWA only.** No notification actions, widgets or Keychain on iOS; see
  above. Kept as a complement, not instead.

## Code shared with the Mac app

The dashboard's per-host `queue` and `history` are the daemon's
`decision_queue` / `decision_history` documents, byte for byte the shapes
Tend for Mac already decodes. Measured with `swiftc -typecheck` against the
iOS 17 simulator SDK: these compile for iOS unchanged once separated from
Mac-only neighbours:

| Shared (moved to `apps/shared/Sources/`) | From |
| --- | --- |
| `TendDecision`, `TendEvidence`, `TendHumanRequestEntry`, `TendAgentWaitingEntry`, `TendPendingItem`, `TendPendingQueue`, `TendVerification`, `TendHumanAnswerEntry`, `TendDecidedItem`, `TendDecisionHistory`, `TendRecordText` | `DecisionQueue.swift` (all but `FleetDecision`) |
| `TendReviewDisposition`, `TendReviewItem` | `FleetReview.swift` |
| `TendLenient` | `TendIPC.swift` |

Not shared, and why: `FleetDecision` (Home's aggregation, needs the IPC
client's host and mailbox types); `TendIPC.swift` (unix socket, and its
response type pulls in every other IPC family); the views (bound to
`AppModel` and AppKit); `TendTheme.swift` (`NSColor`); `BrandMark.swift`
drawing (`NSBezierPath`; the geometry is portable and moves in phase 3 when
the iOS app needs more than the static mark asset). `AgentStatus.swift`
depends on `TendAgent`; the iOS app needs only the lane words and state
colors, which it takes from the brand spec (`assets/brand/tend/STATES.md`).

The shared folder is compiled into both targets as plain sources (no Swift
package, so no `public` churn across 59k lines). The extraction PR only
moves declarations between files and adds the folder to the Mac
`project.yml`; it changes no code. The decoding tests the Mac app already
has keep covering the moved types.

New iOS code: the HTTP client for the decision routes, the Keychain store,
pairing, and the views. About 1,500 lines for phase 1.

**Maintenance cost.** A new field in the decision queue lands in Rust, the
web page, once in the shared Swift models, and in each client that draws
it: sharing the models does not make a card display anything. What keeps a
second and third client from tripling the work is scope, not the folder
layout: the iOS app (and, by the same rule, the Windows stream's app)
follows the decision loop and read-only status only, the shared folder stays
free of AppKit and IPC, and the recorded fixtures plus the iOS CI job make a
model change that breaks a client fail where it was made. Features outside the decision loop (settings, automation,
terminal, start-work editors) never land on iOS. The iOS app is held to
the decision loop plus read-only status, so it does not multiply the work
of every Mac feature.

## Plan

**Phase 1: decide from the phone** (this wave). The owner's brief fixes
it: done when, in the simulator on `tend-test-iphone` and `tend-test-ipad`,
the app pairs with a scratch daemon, shows what needs the person per host,
answers a question and decides a result through the same audited routes the
web uses, and shows the outcome under Decided; with XCUITests for that flow.
It proves the infrastructure (credential, transport, shared models) and the
decision views a notification will open; it does not yet prove the main
improvement, which is phase 2. To keep it small, the cards show what
`/needs` shows (question, options, recommendation, evidence as text and
links, tend's own checks), not every Mac affordance, and the iPad layout is
a plain split view.

1. Server: device pairing and device credentials, the access dialog's
   **Devices**, docs. Rust tests for scope (every other operator route
   refuses a device token), single issue under racing polls, expiry (idle
   and absolute), removal, audit actor, the store created only at the first
   pairing.
2. Mac app: mechanical extraction of the shared models into `apps/shared`.
3. iOS app: pairing (address, code, waiting for approval), Needs you (host
   sections; question / waiting agent / result cards), answer / decline,
   option / Looks good / Send back / Withdraw, Decided tab, Sign out; iPad
   split view; the iOS CI job. Unit tests decode responses recorded from a
   real scratch daemon (kept as fixtures next to the shared models, so the
   Mac tests can use them too), including an unknown item kind and an
   unavailable Fleet host; XCUITests drive a scratch daemon seeded with a
   question and a result.

**Phase 2: notifications.** First a real-device vertical slice (needs
decisions 1 and 4): a question on a host, a content-free push, unlock, the
item opens, answer, Decided. Then actionable notifications with Face ID
and the badge count; `simctl push` tests. In parallel, a short PWA + Web
Push spike on `/needs` to compare for people without an Apple team.
Optional: RFC 8252 sign-in that mints the same device credential.

**Phase 3: the rest of Home, read-only first.** In progress / finished
lanes, hosts, a widget and Lock Screen count, App Intents; read-only pane
output behind a separate, explicit device scope.

Not planned: terminal, settings, automation, account swap, files, changes,
start-work editors.

## Follow-ups outside this stream

- Web dashboard: manifest, icons and `apple-mobile-web-app` tags for
  `/needs`, and Web Push with VAPID for people without an Apple team.
