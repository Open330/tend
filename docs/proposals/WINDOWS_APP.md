# Tend for Windows

Status: proposal (wave 5, 2026-11-15). Phase 1 is being built in the same
wave; this file records the decision and the plan, and is folded into
`docs/WINDOWS.md` when phase 1 ships.

## Owner decisions

Each item has a recommendation and the default this work proceeds on until
you decide. Nothing here blocks phase 1.

| # | Decision | Recommendation | Default meanwhile |
| --- | --- | --- | --- |
| 1 | **Code signing.** Unsigned executables get a SmartScreen "unknown publisher" warning on first run when downloaded (not when built with `cargo install`). Options: Azure Trusted Signing (about USD 10 a month, identity validation of the owner or an organisation), an OV/EV certificate (about USD 200-400 a year plus a hardware token), or none. | None until there is a prebuilt download. Then Azure Trusted Signing under Open330. | Unsigned; installed from source with `cargo install`, like `tend` and `tendd` on Windows today. |
| 2 | **Distribution listing.** winget manifest (free, a PR to `microsoft/winget-pkgs` under a publisher name), MSIX/Microsoft Store (a Partner Center account, about USD 19 one-off for an individual, signing required). | winget after signing exists; no Store. | No listing. |
| 3 | **App identity** (cannot be changed later without users losing their notification settings and the start-with-Windows entry): the AppUserModelID toasts are sent under, and the `HKCU\…\Run` value name. | `dev.tend.windows` (same root as the Mac app's `dev.tend.mac`), display name "Tend". | `dev.tend.windows`, "Tend". |
| 4 | **Windows CI job** (asked earlier, not yet decided). The repository is public, so GitHub-hosted `windows-latest` runners cost no money. The cost is wall time (a cold Windows build of the workspace takes several minutes longer than Linux) and that the job is red until the remaining Windows test fixtures are fixed (45 failing tests on 2026-11-15, being fixed in a separate PR). The phase 1 app test needs an interactive desktop, which hosted runners have, but toasts and WebView2 there are unverified. | Add `cargo clippy` + `cargo test -p tend-app` on `windows-latest` once the fixture PR lands; keep the desktop end-to-end test on `jiun-home`, run by hand before releases. | No job added; the end-to-end script runs on `jiun-home`. |

## What the Mac app does today

From the code (`apps/tend-macos/Sources`, 119 files, about 59,000 lines of
Swift), not from the docs:

| Area | Entry | Data |
| --- | --- | --- |
| Menu bar item, popover, peek panel | `MenuBarAgentController.swift` | `fleet_snapshot` / `fleet_subscribe` over the local socket |
| Home: Your turn / In progress / Finished | `FleetHome.swift` (`FleetLane`, `FleetBoard.works`), `FleetHomeView.swift` | Fleet snapshot, `decision_queue`, `work_backlog` |
| Decisions and review | `DecisionQueue.swift`, `FleetDecisionViews.swift`, `FleetReviewView.swift`, `FleetDecisionHistoryView.swift` | `decision_queue`, `review_dispose`, `decision_history` |
| On-screen choices, replies | `FleetChoiceView.swift`, `MenuBarReplies.swift` | `fleet_command` capture / `send_keys` / `send_prompt` |
| Notifications (Open, Mark as read, Reply…, Switch account…) | `AgentNotifications.swift`, `AgentAttentionCenter.swift` | computed in the app from snapshot changes |
| New agent, start Work, dispatch, pipelines | `NewAgentView.swift`, `WorkConsoleViews.swift`, `FleetDispatch*.swift`, `Pipeline*View.swift` | `agent_start`, `work_up`, `work_command` |
| Panes and terminals | `PaneCaptureView.swift`, `TerminalPaneModel.swift` (Ghostty) | `fleet_command` capture; tendd-owned PTYs |
| Changes (git review), artifacts, Ask, automation, accounts (aas), usage, snapshots | `Changes*.swift`, `Artifact*.swift`, `Ask*.swift`, `Automation*`, `Aas*`, `TendUsage*`, `SessionSnapshot*` | local `git`, `ssh … python3`, Keychain, `aas` CLI, `ask_*` / `automation_*` / `mux_snapshot_*` IPC |
| Settings (General, Notifications, Menu Bar, Accounts, Ask, Agents, Automations, Fleet, Advanced) | `TendSettingsView.swift` | `config_read` / `config_write` |
| Daemon management, onboarding, command palette, global hotkey | `DaemonManager.swift`, `Onboarding*.swift`, `CommandPaletteView.swift`, `TendGlobalHotKey.swift` | starts a bundled `tendd` when the socket does not answer |

Two facts from the code shape everything below:

- **The Mac app makes no HTTP calls and holds no Fleet credential.** Every
  remote operation is a request to the local tendd (`fleet_command`), and
  tendd reaches the other hosts over SSH (`fleet_manager.rs`). So any client
  of the local tendd gets the whole Fleet for free.
- **The Mac app has no login item.** "Start with the session" is new work on
  both platforms.

## What already runs on Windows

`tend` and `tendd` run natively through rmux (`docs/WINDOWS.md`), including
the daemon, hooks, Fleet with Windows as the controller, and the web
dashboard with its OIDC sign-in and pane sharing. Checked on `jiun-home` on
2026-11-15: Windows 11 Education, WebView2 Runtime 157 preinstalled (it ships
with Windows 11), OpenSSH client and server, `ssh localhost` with key auth.

The web dashboard served by tendd already has the decision loop:
`/needs` lists, per host across the Fleet, the questions agents asked,
the agents waiting in their pane and the results filed for review, and
answers or decides them through `POST /api/decisions/requests/{id}` and
`POST /api/decisions/reviews/{id}`. These are the same daemon paths as the
Mac app (`decision_queue`, `review_dispose`), audited to
`dashboard-decisions.jsonl` with the actor. The owner uses this page from the
phone today.

What Windows lacks is the app around it: presence in the tray, a window that
opens to Home, notifications when something needs a decision, and starting
with the session.

## Choice: a small Rust tray app that hosts the dashboard in WebView2

`crates/tend-app`, a Windows binary (`tend-app.exe`) in the workspace:

- **Tray icon** with the number of items that need you, and a menu: Open
  Home, Notifications on/off, Start with Windows, Start tendd (when it is
  down), Quit.
- **Home window**: a WebView2 window on the dashboard's `/home` page,
  served to the WebView by the app itself from a verified connection to
  tendd (see Transport). Closing the window destroys the WebView, so an
  idle app costs a few MB.
- **Toasts**: Windows notifications for new items in the decision queue
  (local and Fleet hosts); clicking one opens Home on that card. Rules in
  Phase 1 below.
- **Start with Windows**: a value under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` that starts the app in
  the tray; on start it checks the daemon and runs `tend daemon start` when
  the dashboard does not answer.

The Home page itself is web: `/home` in the dashboard, built on the `/needs`
cards. The same page serves the browser, the phone and the Windows window.

### Measured

Prototype on `jiun-home` (tao 0.34 + wry 0.53 + tray-icon 0.21 +
tauri-winrt-notification 0.7, release, `opt-level = "s"`, LTO, stripped),
measured with `Get-Process` over the process tree, 15-20 s after start, in
the interactive session:

| Build | Executable | Private memory | Working set | Processes |
| --- | --- | --- | --- | --- |
| Rust tray, no window (the idle state) | 0.78 MB | **2.6 MB** | 20.3 MB | 1 |
| Rust tray with the WebView2 window open on a page | (same) | 171 MB | 339 MB | 7 (1 + 6 `msedgewebview2`) |
| C# .NET 8 WinForms tray, no window (framework-dependent) | 0.2 MB + .NET 8 Desktop runtime | 11.4 MB | 43.6 MB | 1 |
| for scale: `tend.exe` / `tendd.exe` release on the same host | 36.9 / 35.6 MB | — | `tendd` 47 MB | — |

The idle tray is what runs all day; the Rust tray used a quarter of the C#
one's private memory and needs no runtime install. These are prototype
numbers without the app's polling; phase 1 measures the real app (idle with
polling, Home open on real data, after closing and reopening Home) and
records them in `docs/WINDOWS.md`. The WebView2 row was measured with wry
only; other WebView2 hosts were not measured.

### Alternatives rejected

| Option | Why not |
| --- | --- |
| **WinUI 3 native rewrite (C# or C++)** | Re-implements Home, decision cards and the Fleet model in XAML: a third implementation of every Home change (Swift, web, WinUI), in a stack nobody in this repo maintains. WinUI could also host the same web page; then it is the same design as this one in a second language, with the Windows App SDK runtime to ship (unpackaged and self-contained deployment exist). Not measured: the WinUI templates are not installed on `jiun-home`. The choice rests on the repository being Rust, the shared web UI, and the measured Rust footprint, not on a WinUI number. |
| **Rust-native UI (egui, iced, Slint)** | Light, but again a second UI for Home in a toolkit no other client uses; text-heavy cards with Markdown evidence, forms and IME input are what a web view does well and immediate-mode toolkits do poorly. |
| **Full Tauri app** | Tauri is wry + tao plus a bundler, an IPC bridge, plugins and a JS build. It would give single-instance, autostart, notifications and an updater ready-made; with raw wry this project owns navigation, activation, single instance and (later) the installer. We take that cost for phase 1 because each is small (below) and the app needs a custom-protocol proxy Tauri would not remove. Revisit when packaging (decision 2) is decided; a move is not free. |
| **Electron** | Ships its own Chromium (an estimate of 100 MB+ download and idle, not measured here) for what WebView2 already provides on every Windows 11 machine. |
| **No app, browser bookmark / Edge `--app` window / installed PWA** | The baseline, and it works today. But no tray, no toast unless the page is open, no start with Windows, and the token lives in the person's main browser profile. A PWA's web push needs a push service and a public origin, which a loopback dashboard does not have. |
| **App talks to tendd over the named pipe (like the Mac app)** | The page would need a second API, and a pipe does not authenticate its server either (see Transport). The dashboard HTTP API already has the reads and the audited decision writes. |

### Shared code

- **Home and decisions**: one web implementation for the browser, the
  phone and Windows. The Windows-specific code is the tray, the window, the
  toast and the Run key: a few hundred lines of Rust.
- **The Mac app keeps its native Home.** Its lane logic (`FleetBoard`, about
  860 lines including helper folding) is not ported line by line; phase 1's
  web Home uses the same three lanes and words with a simpler unit rule
  (below) that is honest about what it leaves out. Phase 2 moves the
  classification into the daemon (`home_board`, below) so Mac, web, iOS and
  Windows render one model. That is the answer to "a third app must not
  triple the work": classification and dedup live in tendd, each client
  only lays it out.
- **Nothing Swift is shared** with Windows; nothing in `apps/tend-macos`
  changes.

## Transport and sign-in

The app talks only to the **local** tendd's dashboard on loopback
(`[dashboard] bind`, default `127.0.0.1:7878`). Other hosts are reached the
way the Mac app reaches them: through that tendd's Fleet (SSH, the owner's
existing keys). The app opens no new remote surface.

The threat that decides the design (from the review of this proposal): a
loopback address authenticates nothing. While tendd is stopped, **another
local user can bind `127.0.0.1:7878`**, serve a page that looks like Home,
and read whatever credential the app hands that address. Host checks,
CORS, navigation pinning and CSRF proofs all assume the server is genuine.
So the app never sends the token, and never runs a page, before the server
has proved it is tendd:

- **Server proof.** A new unauthenticated route, `GET /api/proof?nonce=N`,
  answers `HMAC-SHA256(token, "tend-dashboard-proof/v1\n" + N)` for a
  fresh random nonce from the app. Only a process that knows the token can
  answer; the answer reveals nothing about it. The app checks it on every
  TCP connection it opens, before the first request on it that carries the
  token, and keeps that one connection alive: a TCP connection cannot be
  taken over by another user's process, so every later request on it goes
  to the same verified tendd. A failed proof drops the connection, sends
  nothing, and the tray says "the dashboard on 127.0.0.1:7878 is not
  tendd".
- **The token stays native.** The WebView loads Home from an app-owned
  origin (`http://tend.localhost/`, wry's custom protocol), and the app
  answers each of its requests itself, by forwarding it over the verified
  connection with `Authorization` added. The page's JavaScript never sees
  the token; nothing is written to `localStorage`. The page's HTML and
  scripts come from the verified tendd too, so an impostor's content is
  never executed.
- **Scoped forwarding.** The app forwards only what Home uses: `GET /home`,
  `/needs`, `/static/*`, `/api/health`, `/api/fleet`, `/api/decisions`,
  `/api/decisions/history`, and `POST /api/decisions/requests/{id}` and
  `/api/decisions/reviews/{id}`. Pane prompts, Fleet commands, terminal
  input, work control and operator administration are refused (403) even
  though the token would allow them. So a bug in the page, or a page that
  is not Home, can at worst read Home's data and answer or decide items,
  each of which is audited (`web:token`).
- **Where the token comes from**: `TEND_DASHBOARD_TOKEN`, else
  `[dashboard] token` in the config file tendd reads. That file is in the
  person's profile; a process that can read it can already drive tendd
  over the named pipe. When the proof fails because the token changed (a
  rotation), the app reads the config again once before reporting it.
- **Navigation**: only the app origin's Home paths load in the window. A
  link elsewhere (the cards' evidence links open with `target=_blank`) is
  refused in the WebView, and an `http`/`https` URL is opened in the default
  browser after parsing; any other scheme (`file:`, custom protocols) is
  dropped. Pane links ("open the pane") open the full dashboard in the
  default browser, where its own sign-in applies.
- **WebView2 profile**: `%LOCALAPPDATA%\tend\webview2`, separate from the
  browser's; it holds no credential (above), so it is not a secret store.
- **Requirement**: the dashboard enabled with token auth on loopback
  (`tend init --component dashboard`). The app refuses `auth = "none"`, an
  unset token and a non-loopback `bind`, and says how to fix each.
- **Why not the named pipe**: a pipe has the same squatting problem (first
  creator wins, `transport.rs` opens by name and cannot check the server's
  identity without an unsafe call), and the Home page would need a second
  API. The proof works over TCP with safe code and keeps one API.

Checked in the code: the `Host` guard wraps assets and API
(`dashboard/server.rs`), there is no CORS layer, the bearer compare is
constant-time (`dashboard/auth.rs`), cookie writes need the exact origin and
`X-Tend-Operator: 1`, and pages and assets carry `frame-ancestors 'none'`
and `X-Frame-Options: DENY`. None of these authenticates the server, which
is what the proof adds.

Not in phase 1: connecting the app to a **published** dashboard
(`https://tend.jiun.dev` with OIDC) instead of a local tendd. That would let
a Windows machine without tend installed act as a pure client; it needs
the OIDC flow in the WebView and is a phase 2 candidate.

## Distribution and updates

Phase 1: `cargo install --path crates/tend-app --locked` next to `tend` and
`tendd`, then `tend-app --install` (writes the Run value and the
notification registration, starts the app) and `tend-app --uninstall`
(removes both). `tend upgrade` learns to rebuild it in phase 2. A prebuilt,
signed download and winget wait for owner decisions 1 and 2.

## Phases

### Phase 1: the decision loop, daily (this wave)

Done when, on real Windows (`jiun-home`):

1. the app starts with the session (Run value) and sits in the tray;
2. Home shows **Your turn / In progress / Finished** from the local daemon
   and the Fleet hosts;
3. a toast appears when something new needs a decision;
4. the person can answer a question and decide a result from Home, through
   the audited dashboard routes, and see the outcome;
5. an automated test of that flow runs on `jiun-home`
   (`scripts/windows-app-e2e.ps1`): a scratch tendd with a Fleet host (a
   second scratch tendd reached over `ssh localhost`), each watching a
   private rmux server through `TEND_RMUX_SOCKET` (a scratch daemon sees no
   rmux pane otherwise, so its agents could not ask anything), seeded
   question and review item, the app started as the Run value starts it, the toast
   recorded, the page driven through Windows UI Automation (the
   accessibility tree WebView2 exposes, so no debugging port is opened),
   and the decisions read back from both daemons. If UI Automation cannot
   reach the page, the fallback is a test-only flag that opens WebView2's
   DevTools port for that run alone, with a scratch token and profile,
   refused unless the config is a scratch one; the installed app never
   opens it. A short manual checklist on the real desktop covers what the
   script cannot: an actual logon, clicking a real toast into the running
   app, and an item decided from the phone disappearing.

Home's lanes in phase 1, computed in `home-model.mjs` from `/api/decisions`
and `/api/fleet`:

- **Your turn**: the decision queue, all hosts (questions, waiting agents,
  results to review), oldest first, with the `/needs` cards and their
  actions.
- **In progress**: live agents that are working or starting, per host,
  newest first. One row per agent (the unit is the agent, id
  `host:session`), not per Work.
- **Finished**: what was decided (`/api/decisions/history`), then agents
  that went idle in the last 24 hours after doing something. "Decided" and
  "agent finished" are labelled as such: a decided question does not mean
  its Work is done.

What this does not cover, stated on the page and here: a Work whose
pipeline stage is blocked or failed while no agent is waiting is in the
Mac app's Your turn (`FleetHome.swift`) but not in the decision queue, so
the web lanes miss it; one Work can show an agent in In progress and
another in Finished; helper folding and the stage strip are absent. The
full dashboard board (`/`) shows blocked stages. Closing that gap is what
`home_board` is for.

Toasts:

- one per new queue item, keyed by host + kind + id (a review item also by
  its generation), remembered across restarts in
  `%LOCALAPPDATA%\tend\app-state.json`;
- at start, items already waiting give one summary toast ("3 things need
  you"), not one each;
- an item that leaves the queue (decided here, on the phone or on another
  host) removes its toast from the notification center;
- the queue is read every 15 s while the dashboard answers, and at once on
  resume from sleep; when it does not answer, the read backs off to 60 s
  and the tray shows the daemon as unreachable, never an empty, healthy
  queue; a host the queue lists as unavailable is named in the tray;
- one instance per user: a second start (the Run value plus a manual
  start) hands "open Home" to the running one and exits;
- clicking a toast opens Home on that card; the card is read again before
  anything is sent, so a stale toast cannot act on a decided item.

Not in phase 1: starting work, pane output, settings beyond the tray menu.

PRs, smallest first: this proposal; the `/home` page and model (web, with
node tests, useful on every platform); `crates/tend-app` (tray, window,
toast, Run value) with unit tests; the end-to-end script and the
`docs/WINDOWS.md` section.

### Phase 2: start and follow work

- Start work and agents from Home (`/api/work-control/up` exists and is
  gated by `allow_work_start`; agent start needs a dashboard route).
- Pane output in Home (the dashboard's pane drawer and output stream
  exist), "Open in terminal" (`wt rmux attach`).
- Toast actions: answer a question's options or reply without opening the
  window.
- `home_board` read in tendd, the gate before any client drops its own lane
  logic: versioned behind a capability string, stable unit ids, a reason
  code per unit (question, waiting agent, review, blocked stage, running,
  finished), host availability, timestamps, membership and a revision;
  fixtures shared with the Swift tests before the Mac app switches. The
  blocked-stage gap above closes here.
- Optional client mode for a published dashboard (OIDC).
- `tend upgrade` rebuilds `tend-app`; signing and winget if decided.

### Phase 3: settings and the rest

Settings in the dashboard (it deliberately has no config-writing API today;
that is a security decision of its own), usage and accounts, snapshots.
Changes (git review), artifacts, Ask and automation editors stay Mac-only
unless use on Windows shows up.

## What Windows cannot carry, or not yet

| Mac feature | Windows |
| --- | --- |
| Native embedded terminal (Ghostty) | rmux panes open in Windows Terminal; the web pane drawer shows output |
| Global hotkey, command palette | possible (`RegisterHotKey`), not planned before phase 3 |
| Notification "Reply…" text field | Windows toasts support inputs; phase 2 |
| Keychain for Ask keys | Credential Manager; only if Ask comes to Windows |
| Account swap | `[[accounts.*]]` is refused on Windows today (`docs/WINDOWS.md`) |

## Risks

- **WebView2 absent or broken** (Windows 10 LTSC, removed runtime): the app
  says so and offers to open Home in the default browser instead.
- **Dashboard off**: the app cannot show Home; it says which command
  enables it. Toasts need the dashboard too.
- **Toast identity**: an unpackaged app shows toasts under a registered
  AppUserModelID (`HKCU\Software\Classes\AppUserModelId\<id>`); clicking a
  toast after the app exited does nothing in phase 1 (no COM activator).
- **Polling**: toasts come from reading `/api/decisions` every 15 s; that
  read already serves `/needs` every 10 s per open page. With Fleet hosts it
  reaches each host's relay; the phase 1 measurement includes it.
- **Proof route**: `/api/proof` is a new unauthenticated route. It answers
  only in token mode, with an HMAC of a caller-chosen nonce under a
  domain-separated label, so it is not an oracle for anything else the
  token signs (nothing else is signed with it).
