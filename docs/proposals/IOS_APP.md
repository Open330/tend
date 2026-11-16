# Tend on iPhone and iPad

Status: phase 1 built (wave 5, 2026-11-15); wave 6 adds the whole Home and
a wider, opt-in device scope (2026-11-16), see
[Wave 6](#wave-6-the-whole-home-and-a-wider-device-scope) and [Plan](#plan).

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
| 7 | Let a paired phone see hosts and agents, read agents' recent output and start new agents (the `operate` scope). The token is an ordinary bearer credential: whoever holds it (a phished approval, a stolen phone) can start agents with bypassed permissions on every covered control-mode host until it is removed or expires. | Yes, accepting that risk, opt-in per device: chosen on the phone, looked up then ticked separately by the operator, create-only starts in projects agents already use, audit before dispatch, **Limit to deciding** and Remove. | Built and tested against a scratch daemon only; existing phones stay decide-only until paired again. |
| 8 | Turn on `[dashboard] allow_work_start = true` on jiun-mini. It is off today, so neither the web nor the phone can start work through the dashboard. | Turn it on only after the start-work slice (plan step 4) has passed its tests, and when the owner wants to start work from the phone. Tests on a scratch daemon are not a reason to turn it on. | Off. The app shows "Starting work is turned off on this dashboard" and keeps the rest. |
| 9 | Expiry for `operate` devices. A token in daily use lasts the full lifetime whoever uses it. | The same as `decide` (30 days unused, 90 days in all); shorter would mean pairing the phone again every few weeks. Removal is the real defence. | 30 / 90 days. |

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

## Wave 6: the whole Home, and a wider device scope

Build 0.1.0 (1) went to TestFlight on 2026-11-16. The owner's verdict: the
type and layout look raw, only **Your turn** is visible (no hosts, no other
lanes), and new work cannot be started. Phase 1's "decide only" is not what
the owner wants to use, so phase 3's read-only Home and a start-work slice
move ahead of push. Push stays phase 2 in substance and follows.

### What the app shows

The same Home as Tend for Mac, in its words and order (lanes from
`FleetLane` in `apps/tend-macos/Sources/FleetHome.swift`, rules shared with
the web Home's `home-model.mjs`):

- **Your turn**: the decision queue (`/api/decisions`), as in phase 1.
- **In progress**: agents that are working or starting, per host.
- **Finished**: decided items (`/api/decisions/history`) and agents that
  went idle in the last 24 hours.
- **Hosts**: every Fleet host with its state, the agents on it, and an
  agent's recent output on request.
- **New work**: host, project, request, Start.

The lanes, hosts and agents come from the Fleet snapshot the web Home
reads (`/api/fleet`), served to a device through its own cut-down route
(below). On iPad: a sidebar (Home, each host, Settings) and the
three lanes side by side, as on the Mac. On iPhone: Home has a lane strip at
the top with the three lanes and their counts, always visible; tapping a
lane or swiping sideways switches the list below. A count is the at-a-glance
part (what needs me, how much is running); three stacked lists would push
Finished off the screen and a tab per lane would hide the counts. Hosts and
New work are tabs of their own.

Visual parity is held to a test, not a description: the Mac app and the iOS
app are screenshotted side by side for each screen (iPhone and iPad, light
and dark) and a reviewer names any place iOS looks rawer. The design
tokens (state colours, type roles, spacing, radii) come from one file
shared with the web Home, not a second copy.

### The scope change

A phase 1 device token reaches only the decision routes. Hosts, agents,
output and starting work need more, and **an existing token is never
widened**: a device keeps the scope it was approved with, or less.

**Two scopes.** Every device has exactly one:

| Scope | What it adds | What it means |
| --- | --- | --- |
| `decide` (phase 1, the default) | `GET /api/decisions`, `GET /api/decisions/history`, `POST /api/decisions/requests/{id}`, `POST /api/decisions/reviews/{id}`, its own `GET /api/device`, `POST /api/device/forget` | Read every covered host's questions and evidence and answer them. |
| `operate` (new, opt-in) | everything in `decide`, plus four device-only routes: `GET /api/device/fleet`, `POST /api/device/capture`, `GET /api/device/work-options`, `POST /api/device/work` | Also see every host and its agents, read an agent's recent output, and **start new agents, which run with permissions bypassed on any covered control-mode host**. |

`operate` is remote execution on the owner's hosts by whoever holds the
token. Face ID on the phone (below) protects against someone using the
unlocked app; it does not protect the token itself, which is an ordinary
bearer credential like the dashboard token. A token phished at approval or
lifted from the phone can start agents until it expires or is removed.
Owner decision 7 is whether to accept that.

**Device-only routes, not the operator ones.** The operator routes are
wider than the phone needs, and filtering them per caller would be policy
hidden in handlers:

- `POST /api/work-control/up` is not "start new work": given an existing
  Work id it reconciles and re-prompts that Work's agents
  (`work_up::apply_durable`), and it accepts `pipeline`, `skill`,
  `context`, `external`, `no_ticket`, `dry_run` (which is not
  side-effect free: ticket lookup, worktree creation) and any `cwd`.
- `POST /api/fleet/{host}/command` takes any Fleet operation.
- `GET /api/fleet` carries SSH targets, labels, capabilities and every
  pane, and must keep its anonymous behaviour in `public_read` mode.

So `operate` reaches none of them. It gets four exact routes, each with its
own typed request (`deny_unknown_fields`), its own minimal response, and
`Cache-Control: no-store`. They live in a router of their own behind an
`operate` gate, are matched as exact paths (never by prefix: `/api/device`
also prefixes `/api/devices`), and change neither the operator routes nor
`operator::authenticate`.

- `GET /api/device/fleet`: the same JSON shape as `/api/fleet`, so one
  decoder reads both, cut to what the app shows: per host the alias,
  local, mode, state, version, last error and last seen; per agent its
  kind, session id, pane, state, title, recap, last prompt, response and
  notification, folder, model and times. No SSH target, labels,
  annotations, capabilities, backends or agentless panes.
- `POST /api/device/capture {host, session_id}`: the recent output of the
  pane of **one agent in the current snapshot**, named by its agent session
  id. The server builds the exact `PaneKey` from the snapshot and runs
  exactly `FleetOperation::Capture` through the Fleet manager, so the
  configured-host lookup, `[fleet] capture_policy` (`never` refuses, as for
  operators) and the observe/control checks are the existing ones. A pane
  with no agent cannot be named. The answer is the sanitized text only (no
  raw base64), at most 400 lines and 64 KiB; at most one capture per device
  in flight. Each capture, refused or not, is an audit line.
- `GET /api/device/work-options?host=` runs exactly
  `tend work options --json` (no stdin, no other argv) on that host and
  answers only: whether starting work is allowed here (`allow_work_start`),
  the pipelines by name and description, and the project folders this
  phone may choose (below). No config path, prompts or provider options.
- `POST /api/device/work {idempotency_key, host, project, pipeline, request}`
  starts **new** work, and only that:
  - **Create-only by construction.** The server mints the Work id
    (`phone-MMdd-` plus 12 random characters); the phone cannot name one, so
    it cannot reach an existing Work. The server also refuses if that id is
    already known on the target, before dispatch.
  - **Fixed launcher input.** The server builds the `WorkUpRequest` itself:
    the minted id, the request text as `body`, the chosen `pipeline`
    (must be one `work-options` listed for that host), `cwd` = the chosen
    project, `host`, `no_ticket = true`, `dry_run = false`, and nothing
    else (no workspace, skill, context or external reference).
  - **A project is required, from the server's own list.** `project` must
    be, byte for byte, one of the folders `work-options` listed for that
    host: folders an agent on that host has worked in, as the current
    snapshot shows. There is no empty or default project: with no `cwd`,
    `tend work up` would fall back to its own working directory
    (`resolve_cwd`), which no one chose. If the host's config has a route
    that would match the minted id and set its own `cwd`, a worktree or a
    `prepare` command, the start is refused before dispatch rather than
    letting the route move the agents elsewhere; the minted prefix
    `phone-` is chosen so ordinary routes do not match it. The folder the
    agents get is recorded in the audit line. (A folder limits where agents
    start, not what an agent with bypassed permissions can reach.)
  - **The existing gates still apply:** `[dashboard] allow_work_start` on
    the dashboard host (default off; off on jiun-mini today, see owner
    decision 8) and a control-mode target host.
  - **Idempotent, durably.** `idempotency_key` is a UUID the app makes per
    Start. Before anything is dispatched, the server **reserves** it in the
    device store (a `device_starts` table, unique on device id and key):
    the request's fingerprint (SHA-256 of host, project, pipeline and
    text), the minted Work id and the state `reserved`, written with
    `synchronous=FULL`. Only then is `tend work up` run; the state becomes
    `started` or `failed` with the operation id when it answers, or
    `unknown` when it times out. A retry with the same key and fingerprint
    returns that entry's state and never dispatches again; the same key
    with another fingerprint is refused (409); a concurrent duplicate sees
    the reservation and gets its state. After a restart, every entry still
    `reserved` is marked `unknown` and is **never dispatched again**: a
    second `work up` with that id could now find the Work the first one
    created and re-prompt it. An `unknown` start is shown as such on the
    phone and in the audit; a person checks the host. Entries are kept for
    7 days and an unresolved one is never dropped before it is resolved
    (removing the device removes its entries). One start per device in
    flight; at most 20 per device per hour.
  - **Audited before and after.** An intent line (device id and label,
    scope, approver, idempotency key, fingerprint, host, project, pipeline,
    minted Work id, request length but not its text) is written **before**
    dispatch; if it or the reservation cannot be written, nothing starts
    (503). A result line follows with the outcome (started, refused,
    failed, unknown) and the operation id.

**Granted at approval, not by the device.**

1. In the app, the person chooses "Decide only" or "Decide, see hosts and
   start work" before pairing; the request carries that scope, fixed for
   the life of the request.
2. The access dialog no longer approves on the code alone. Entering the
   code first **looks it up without granting anything** (`POST
   /api/devices/lookup {user_code}`: name, platform, requested scope, when
   it asked). The operator then approves that exact request
   (`POST /api/devices/approve {user_code, scope}`). For `operate` the
   dialog shows, unticked: "Also let this device see every host and agent,
   read their recent output, and start agents that run with permissions
   bypassed on every covered control-mode host. Only tick this for a code
   on a device you are holding right now." Approving without the tick
   issues `decide`.
3. The issued scope is the lesser of what was asked and what was granted;
   the token response and `GET /api/device` name it, and the app shows only
   what it may do.

To widen a `decide` device the person pairs it again: the app first signs
the old device out on the dashboard (the existing Sign out, which removes
it there), and only when that succeeded starts a new request. There is no
"upgrade this token" or "replace device X" route, so an old key never
becomes a way to get a wider one.

**Narrowing and revocation.** Remove, Remove all and the app's Sign out
work as in phase 1. **Limit to deciding** drops an `operate` device to
`decide` without unpairing it; it is irreversible for that token. It is
serialized with issuance and removal (the `transitions` lock), persisted
before it reports success, and read on every request (the registry is
consulted per request, never a cached session). A request already admitted
when the limit lands may finish; neither narrowing nor removal undoes an
agent already started or what it did.

**Expiry.** 30 days unused / 90 days in all for both scopes (owner
decision 9). Every use counts, so a token in daily use, by the owner or by
someone who stole it, lasts the full 90 days: idle expiry is not a defence
against an actively used stolen token. Removal is.

**On the phone.** Start asks for Face ID or the passcode every time
(LocalAuthentication, `.deviceOwnerAuthentication`). That is protection
for the unlocked phone left on a table, not for the token, as said above.

**Storage and compatibility.** One `scope TEXT NOT NULL DEFAULT 'decide'`
column (schema version 2, added in place on open), so every phase 1 device
stays `decide`; a value other than `decide` or `operate` reads as `decide`.
`/api/access` reports `capabilities.device_scopes`. An older daemon ignores
`scope` in the request and issues a `decide` token; the app reads the
issued scope and says the host needs a newer tend for the rest. A device
without `operate` gets 403 from the four routes (never 401, which the app
reads as "removed"). No IPC or Fleet protocol change.

**Shared with the web Home.** The web Home keeps reading the operator
routes. The design tokens and the lane rules are shared (one tokens file,
one fixture both test against); the device routes are device-only on
purpose, because their limits are the point.

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

**Wave 6: the whole Home** (before push, at the owner's request after
build 1). In order, each its own PR with screenshots:

1. Server, scope plumbing: the `operate` scope, lookup-then-approve with
   the scope tick, Limit to deciding, schema version 2, and
   `GET /api/device/fleet`. Rust tests: the route table under both scopes
   (every operator route still refuses an `operate` token, including the
   generic Fleet command, `/api/work-control/up` and `/api/fleet`), public
   modes unchanged, approval granting less than asked, narrowing racing a
   request, unknown scope values, the v1 store opening as `decide`.
2. App: visual parity and the three lanes, alongside 1. Mac type roles,
   system state colours, lane columns and cards, Markdown where the Mac
   renders it, both colour schemes; the iPhone lane strip and the iPad
   lanes. In progress and the agents half of Finished need `operate`; a
   `decide` device shows Your turn and decided items and says what pairing
   again adds. The coordinator is told when 1 and 2 are on main, for the
   next TestFlight build; that build does not yet start work.
3. Hosts and output: `POST /api/device/capture` and the app's Hosts, host
   and agent screens. Tests: agent-only panes, stale and cross-host
   sessions, `capture_policy = never` locally and remotely, size limits.
4. Start work: `GET /api/device/work-options`, `POST /api/device/work` and
   the app's Start Work with Face ID. Tests: raw bodies with extra fields,
   an empty project, a project not in the list, `..` and symlinked paths,
   a pipeline not listed, a route that would match the minted id with its
   own cwd, worktree or prepare; concurrent same-key requests, a restart
   before dispatch and after dispatch but before the result (never
   dispatched twice), a timed-out retry, conflicting reuse, the audit line
   or reservation failing closed, `allow_work_start` off; an XCUITest that
   starts work on a scratch daemon whose pipeline runs a harmless stand-in
   agent.

**Phase 2: notifications.** First a real-device vertical slice (needs
decisions 1 and 4): a question on a host, a content-free push, unlock, the
item opens, answer, Decided. Then actionable notifications with Face ID
and the badge count; `simctl push` tests. In parallel, a short PWA + Web
Push spike on `/needs` to compare for people without an Apple team.
Optional: RFC 8252 sign-in that mints the same device credential.

**Phase 3: around Home.** A widget and Lock Screen count, App Intents.
(The lanes, hosts, pane output and starting work moved to wave 6.)

Not planned: terminal, typing into panes, settings, automation, account
swap, files, changes, pipeline and preset editors.

## Follow-ups outside this stream

- Web dashboard: manifest, icons and `apple-mobile-web-app` tags for
  `/needs`, and Web Push with VAPID for people without an Apple team.
