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
- **Home window**: a WebView2 window on the local dashboard's new `/home`
  page (below). Closing the window destroys the WebView, so an idle app
  costs a few MB.
- **Toasts**: Windows notifications for each new item in the decision queue
  (local and Fleet hosts); clicking one opens Home on that card.
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

WebView2 costs the same in any shell that uses it (Tauri, wry, C#), so the
web-view choice is about the open window, and the open window is something
the person asked for. The idle tray is what runs all day; the Rust tray uses
a quarter of the C# one's private memory and needs no runtime install.

### Alternatives rejected

| Option | Why not |
| --- | --- |
| **WinUI 3 / C# native rewrite** | Re-implements Home, decision cards and the Fleet model in XAML/C#: a third implementation of every Home change (Swift, web, C#), in a language nobody in this repo maintains. Reaching the Mac app's 59,000 lines is out of reach; reaching the web page's decision loop alone is several thousand lines. Needs the Windows App SDK runtime and, for toasts with identity, packaging. Its idle memory was not measured (the WinUI template is not installed on `jiun-home`); the WinForms measurement above is the floor for the .NET path. |
| **Rust-native UI (egui, iced, Slint)** | Light, but again a second UI for Home in a toolkit no other client uses; text-heavy cards with Markdown evidence, forms and IME input are what a web view does well and immediate-mode toolkits do poorly. |
| **Full Tauri app** | Tauri is wry + tao plus a bundler, an IPC bridge, plugins and a JS build. We need none of the bridge (the page talks to the dashboard API directly) and Tauri's bundler targets NSIS/MSI, which decision 2 defers. Using its two underlying crates keeps the same WebView2 engine with fewer dependencies; moving to Tauri later is mechanical. |
| **Electron** | Ships its own Chromium (about 100 MB+ download, 100 MB+ idle) for what WebView2 already provides on every Windows 11 machine. |
| **No app, browser bookmark / Edge `--app` window / installed PWA** | No tray, no toast unless the page is open, no start with Windows, and the token would live in the person's main browser profile. A PWA's web push needs a push service and a public origin, which a loopback dashboard does not have. |
| **App talks to tendd over the named pipe (like the Mac app)** | Would need a second client of the IPC protocol plus a renderer for the Home model. The dashboard HTTP API already exposes exactly the reads and the audited decision writes, and the window needs the dashboard anyway. One contract (the dashboard API) instead of two. |

### Shared code

- **Home and decisions**: one web implementation for the browser, the
  phone and Windows. The Windows-specific code is the tray, the window, the
  toast and the Run key: a few hundred lines of Rust.
- **The Mac app keeps its native Home.** Its lane logic (`FleetBoard`, about
  860 lines including helper folding) is not ported line by line; phase 1's
  web Home uses the same three lanes and words with a simpler unit rule
  (below). Phase 2 moves the lane classification into the daemon (one read,
  `home_board`) so Mac, web, iOS and Windows render one model. That is the
  answer to "a third app must not triple the work": the model lives in
  tendd, the clients only draw it.
- **Nothing Swift is shared** with Windows; nothing in `apps/tend-macos`
  changes.

## Transport and sign-in

The app talks only to the **local** tendd's dashboard on loopback
(`[dashboard] bind`, default `127.0.0.1:7878`) with the dashboard's bearer
token. Other hosts are reached the way the Mac app reaches them: through
that tendd's Fleet (SSH, the owner's existing keys). The app opens no new
remote surface.

- **Where the token comes from**: `TEND_DASHBOARD_TOKEN`, else
  `[dashboard] token` in the config file tendd reads (`TEND_CONFIG`, else
  the default path). That file is in the person's profile; any process that
  can read it can already drive tendd over the named pipe, so the token
  grants nothing a same-user process does not already have.
- **How the page gets it**: a WebView2 initialization script stores it in
  `localStorage` (`tend.token`, the key the dashboard pages already read)
  only when the document's origin is exactly the dashboard origin. The
  token is never put in a URL or on a command line.
- **Navigation is pinned** to the dashboard origin. Any other link opens in
  the default browser; WebView2's own new windows are refused.
- **WebView2 profile**: `%LOCALAPPDATA%\tend\webview2`, the person's own
  profile directory, not the browser's profile.
- **What protects the loopback port**: the token on every `/api/*` call
  (constant-time compare), the dashboard's DNS-rebinding `Host` check, no
  CORS (a page on another origin cannot send the `Authorization` header or
  read a response), `frame-ancestors 'none'`. Another user on the same
  machine can connect to the port but has no token. This is the same
  exposure the dashboard has for every user today; the app does not widen
  it.
- **Requirement**: the dashboard must be enabled with token auth on the
  Windows host (`tend init --component dashboard` does it). The app refuses
  `auth = "none"` and a non-loopback `bind`, and says how to fix either.
- **Decisions are made as `web:token`** in the audit log, like any token
  user of the dashboard. Phase 2 can add a distinct actor (`windows-app`)
  if the owner wants to tell them apart.

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
   recorded, the page driven through the WebView2 DevTools protocol, and the
   decisions read back from both daemons.

Home's lanes in phase 1, computed in `home-model.mjs` from `/api/decisions`
and `/api/fleet`:

- **Your turn**: the decision queue, all hosts (questions, waiting agents,
  results to review), oldest first, with the `/needs` cards and their
  actions.
- **In progress**: live agents that are working or starting, grouped by
  Work when they belong to one, per host.
- **Finished**: what was decided recently (`/api/decisions/history`) and
  agents that went idle after doing something, newest first.

Not in phase 1: helper folding (peer helpers show as their own rows), the
pipeline stage strip, starting work, pane output, settings beyond the tray
menu.

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
- `home_board` read in tendd; web, Mac and iOS render it.
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
- **Polling**: toasts come from reading `/api/decisions` (every 15 s, and
  at once on a dashboard event); that read already serves `/needs` every
  10 s per open page.
