# tend dashboard

A web UI served by the daemon. Its address (`/`) opens **Home**, the same
lanes Tend for Mac opens on (Your turn, In progress, Finished, and the
backlog), with **Explore** (every host's panes) and **Settings** (access,
paired devices, sharing) beside it; see
[The shell](#the-shell-home-explore-settings). The phone app and the Windows
tray app read the same Home through this server.

The API reads the canonical tend hierarchy: **Workspace → Work → Run →
Agent session**, with tmux session/window/pane retained only as the current
execution binding. An optional Linear/GitHub/Jira issue is a reference
attached to Work; it is not the Work identity itself.

Home and Explore refresh by polling while the page is visible; a pane open in
Explore streams its output. Optional control actions are protected by a
bearer token that can be pasted into the browser like a PAT.

The dashboard is **off by default** and **loopback-only when on by default**.
Token authentication is the default auth mode, and an enabled dashboard must
have an explicit token. Binding directly to a non-loopback address requires explicit public-bind
acknowledgement. For sharing through an HTTPS reverse proxy, keep the daemon
bound to loopback and configure the public origin as described below.

## Surfaces

| Path                | What it returns                                                                |
| ------------------- | ------------------------------------------------------------------------------ |
| `GET /`, `/home`, `/explore`, `/settings` | The shell's pages: Home, Explore, Settings. Someone who may read but not operate gets them read-only; a visitor whose reads answer 401 is sent to `/welcome`. |
| `GET /welcome`      | The landing for visitors asked to sign in: what tend is, the docs, sign-in. `?return_to=<path>` is where sign-in comes back to. |
| `GET /workbench`    | The shell, for links the retired workbench made: its fragment opens in Explore or Messages, anything else goes to Home (see [Old links](#old-links)). |
| `GET /static/*`     | Embedded JS/CSS/font assets.                                                   |
| `GET /api/health`   | `{ ok, version, protocol }`                                                    |
| `GET /api/proof?nonce=N` | No credential: `{ version: 1, proof }`, `proof` = hex HMAC-SHA256(token, `tend-dashboard-proof/v1\n` + node + `\n` + bind + `\n` + N), where node is this host's Fleet node id (`host-id`) and bind the dashboard's address, for a 32-128 hex-digit nonce. A client checks it before sending the token, since anyone can bind a loopback port while tendd is down; node and bind stop an impostor from relaying the nonce to another tend that shares the token. 404 without a token or with `auth = "none"`. |
| `GET /api/access`   | Current read/control access mode and whether the supplied PAT can edit.        |
| `GET /api/agents`   | Current `Store` snapshot.                                                      |
| `GET /api/fleet?selector=...` | Cached hierarchy, including the always-present local node.          |
| `GET /api/panes`    | Global tmux pane list (every readable socket), with per-socket scan errors.   |
| `GET /api/works`    | Canonical v2 Workspace/Work/Run snapshot plus unlinked executions.            |
| `GET /api/work-metadata` | Durable titles, goals, next actions, and manual workflow stages.        |
| `GET /api/collaboration` | Indexed collaboration requests for the node/edge graph and sequence drill-down. |
| `GET /api/terminal-sessions` | Tend-owned PTY sessions.                                           |
| `GET /api/timeline` | Timeline document from `activity.ndjson` plus currently-open agent/tmux spans. |
| `GET /api/events`   | SSE stream: `snapshot` (initial), `transition` (live), `lagged` (backpressure)|
| `GET /api/windows/{window}/layout?socket=` | Pane geometry (`left`/`top`/`width`/`height` in cells, `active`, `zoomed`) for one tmux window. |
| `GET /api/panes/{pane}/output?socket=&lines=N` | Operator only: `{ pane, text, captured_at }`, the pane's newest `N` lines (default 200, max 1000; 64 KiB cap) as plain text, escapes stripped. |
| `GET /api/panes/{pane}/output/stream?socket=&lines=N` | Operator only, SSE: `output` events (`{ pane, text, captured_at }`, the full bounded text) on connect and whenever it changes; `gone` when the pane closes. See [Live pane output](#live-pane-output). |
| `POST /api/panes/{pane}/prompt` | Send and optionally submit text to a pane.                       |
| `POST /api/panes/{pane}/abort` | Send Ctrl-C to a pane.                                             |
| `POST /api/fleet/{host}/command` | Execute a serialized Fleet operation; host mode is rechecked.   |
| `PUT /api/work-metadata` | Save a definition by logical `{workspace_id, work_id}` identity.       |
| `POST /api/work-control/prompt` | Prompt every live agent run linked to one Work. Accepts `text`, or shared-composer `skill`/`body`/`context`. |
| `POST /api/work-control/up` | Start or converge a Work pipeline and optionally link an external issue. Requires `allow_work_start`. |
| `POST /api/work-control/abort` | Send Ctrl-C to every live agent run linked to one Work.          |
| `POST /api/terminal-sessions/{id}/input` | Send input to a Tend-owned PTY.                         |
| `POST /api/terminal-sessions/{id}/terminate` | Terminate a Tend-owned PTY.                          |
| `POST /auth/devices/request`, `POST /auth/devices/token` | A device asks to be paired and collects its token. See [Paired devices](#paired-devices). |
| `GET /api/devices`, `POST /api/devices/approve`, `POST /api/devices/{id}/remove`, `POST /api/devices/remove-all` | Operator only: list, approve, unpair devices. |
| `GET /api/device`, `POST /api/device/forget` | A paired device: who it is, unpair itself. |

`auth = "token"` protects all API endpoints. `auth = "public_read"` leaves
GET/SSE endpoints public but always requires the bearer token for write/control
actions. `auth = "none"` leaves reads public and disables writes entirely.
Collaboration bodies and attached details require an authenticated operator
(valid PAT or operator login) in `token` or `public_read` mode. Anonymous users
and signed-in viewers receive a redacted topology/status summary. `none` mode
always redacts details. History responses use `Cache-Control: private, no-store`.
Static routes are public in every mode — see "Why the HTML is public" below.
Fleet routes always expose the in-process `local` node. `[fleet] enabled =
true` adds outbound SSH hosts; reads reuse the daemon's per-host cache and
never create request-scoped SSH connections. See
[FLEET.md](FLEET.md).

## Quick start

### Recommended: generate and persist a token with `tend init`

```sh
tend init --component dashboard
```

The initializer enables the loopback dashboard, generates a token, persists it
in `~/.config/tend/config.toml`, and prints a one-time bootstrap URL such as
`http://127.0.0.1:7878/#token=...`.

### Manual loopback setup with a token

For a manual setup, provide the token explicitly:

```sh
TOK=$(openssl rand -hex 32)
tendd --dashboard --dashboard-token "$TOK"
# then in the browser:  http://127.0.0.1:7878/#token=$TOK
```

The page captures `#token=...` into `localStorage` and rewrites the URL.
Subsequent requests carry `Authorization: Bearer <tok>`. The token persists
across tab close and browser restart, so you only need to paste it once per
browser profile. To revoke, clear the `tend.token` key (DevTools → Application
→ Local Storage) or restart `tendd` with a different token.

Legacy `?token=...` URLs are still accepted for compatibility and immediately
scrubbed, but fragments are the supported bootstrap format because browsers do
not send them in HTTP requests or `Referer` headers.

A browser that can read nothing (reads return 401: no operator session and no
working token) is sent from `/`, `/home`, `/explore` or `/settings` to
`/welcome` instead of an empty dashboard. That page says what tend is and
offers the ways in: **Sign in** when
[operator sign-in](#operator-sign-in-with-oidc) is configured, or else a
pointer to the dashboard link `tend init` prints. A stored token that the
server rejects is reported there. The address the visitor opened goes along
as `return_to` (a same-origin path, rebuilt from what the page names, so
never with a credential: `#token=`/`?token=` is stored and scrubbed first),
and sign-in comes back to it. `/welcome` is its own page (`welcome.html`,
`welcome.mjs`, `landing.css`) under the same strict policy as Home: no inline
script or style, nothing from other origins. `public_read` and
`auth = "none"` dashboards never send anyone there, since their reads
succeed without credentials.

Someone who may read but not operate (an anonymous visitor with
`auth = "public_read"`, any visitor with `auth = "none"`, or a signed-in
viewer, who gets 403 on operator routes) gets the shell read-only:

- **Explore** lists every host's panes and their state from `/api/fleet`.
  Pane output, the composer and Interrupt are operator routes, so the page
  never asks for them.
- **Messages** shows the collaboration with details redacted, as the API
  already returns it.
- **Home** says it needs an operator and offers **Upgrade with token**, a
  field in the page. Where the dashboard enrolls accounts, a signed-in
  viewer's token goes once to `POST /auth/enroll`, which makes the account
  an operator (otherwise the viewer is told to ask the owner); a browser
  without an account keeps the token as its credential, and only if it
  makes the browser an operator. With `auth = "none"` there is no token, so
  there is nothing to upgrade with.
- **Settings** shows who is signed in, with sign-in, sign-out and the same
  upgrade.

An operator who loses operator access mid-session (signed out, downgraded
to a viewer) loses everything private on the page first: the board,
drafts, open sheets, Explore's output and drafts.
Its dashboard screenshots show the demo GIFs' made-up fleet, captured from a
real tend in the isolated sandbox; regenerate them with
`scripts/landing-shots/live.sh` (tmux, Playwright, Chrome and `cwebp`).
The hero film is captured from a real tend in the isolated sandbox (tmux,
the status line, `tend attend`, `tend peek`) with
`scripts/landing-shots/film.sh`, which writes `landing-frames.mjs`.

### Loopback-only without authentication (dev / single-user)

Disabling auth requires an explicit opt-out:

```toml
# ~/.config/tend/config.toml
[dashboard]
enabled = true
auth = "none"
# bind defaults to 127.0.0.1:7878
```

Or via flags:

```sh
tendd --dashboard --dashboard-auth none
```

Open <http://127.0.0.1:7878/>.

### Public read-only dashboard with PAT editing (LAN / VPN)

This is the mode for a dashboard that anyone on the reachable network may
view, while only a browser with the PAT can display and use edit controls:

```toml
[dashboard]
enabled = true
bind = "0.0.0.0:7878"
allow_public = true
auth = "public_read"
token = "replace-with-a-long-random-token"
```

Open `http://<host>:7878/` for anonymous read-only access: the shell opens
read-only. **Upgrade with token** (on Home, or in Settings) takes the token
and enables prompt and interrupt. The browser stores it in `localStorage`;
**Forget token** in Settings removes it again.

The equivalent CLI invocation is:

```sh
TOK=$(openssl rand -hex 32)
tendd --dashboard \
      --dashboard-bind 0.0.0.0:7878 \
      --dashboard-auth public_read \
      --dashboard-token "$TOK" \
      --allow-public
```

### Public bind with fully token-protected reads (LAN / VPN)

You must opt in to *both* a non-loopback bind *and* a token. Either alone
fails at startup:

```sh
TOK=$(openssl rand -hex 32)
tendd --dashboard \
      --dashboard-bind 0.0.0.0:7878 \
      --dashboard-token "$TOK" \
      --allow-public
```

Open `http://<host>:7878/#token=$TOK` in the browser.

If you skip `--allow-public` or `--dashboard-token`, `tendd` refuses to start
with a clear message — same applies to TOML configs.

### Public bind without API auth or editing

For a trusted private network, you can intentionally expose the read-only API
without a bearer token:

```toml
[dashboard]
enabled = true
bind = "0.0.0.0:7878"
allow_public = true
auth = "none"
```

Or via flags/env:

```sh
tendd --dashboard \
      --dashboard-bind 0.0.0.0:7878 \
      --allow-public \
      --dashboard-auth none
```

This exposes all GET/SSE data to anyone who can reach the port. POST control
routes return `403 Forbidden`, so this mode cannot become anonymously writable.
Use it only on a network you already trust.

> ⚠️ **TLS is out of scope.** Use a reverse proxy (nginx, Caddy, Traefik) to
> terminate TLS in front of the dashboard. Set `proxy_buffering off;` for the
> SSE endpoint or live updates will batch.

## Configuration reference

All knobs live under `[dashboard]` in the TOML config. The CLI / env layer
overrides per-field, with **env > CLI flag > TOML > built-in default** (clap
already enforces env-beats-flag for the fields it covers).

| Key                  | Type    | Default            | CLI / env                                                       |
| -------------------- | ------- | ------------------ | --------------------------------------------------------------- |
| `enabled`            | bool    | `false`            | `--dashboard` / `--no-dashboard` / `TEND_DASHBOARD_ENABLED`     |
| `bind`               | string  | `"127.0.0.1:7878"` | `--dashboard-bind` / `TEND_DASHBOARD_BIND`                      |
| `auth`               | string  | `"token"`          | `--dashboard-auth` / `TEND_DASHBOARD_AUTH` (`token`, `public_read`, or `none`) |
| `token`              | string  | unset (required with `token`/`public_read`) | `--dashboard-token` / `TEND_DASHBOARD_TOKEN` |
| `allow_public`       | bool    | `false`            | `--allow-public` / `TEND_DASHBOARD_ALLOW_PUBLIC`                |
| `pane_cache_ttl_ms`  | u64     | `2000`             | (TOML only)                                                     |

## How "global" works

The daemon's agent registry is already global per user — every tmux server
forwards `AgentEvent`s to the same `tendd`. The dashboard's *new* trick is
the pane scanner: it enumerates every tmux socket under `$TMUX_TMPDIR`,
`/tmp/tmux-$UID/`, and (on macOS) `/private/tmp/tmux-$UID/`, runs `tmux -S
<sock> list-panes -a` against each, and folds the results.

Per-socket failures are captured into `errors[]` rather than blanking the
whole list — a wedged tmux server cannot kill the dashboard's view of healthy
ones. Every per-socket invocation has a 1 s timeout.

Results are cached for `pane_cache_ttl_ms` (default 2 s, lazy pull) so a
hammering refresh loop doesn't fork tmux 60 times a minute.

## Work-oriented projection

`tend::work::build_snapshot` is the single projection used by the HTTP and CLI
dashboards. The browser consumes `/api/works`; it does not infer Work from a
session name, a ticket-shaped window name, or repository cwd.

| Concept | Stable identity / source | Dashboard responsibility |
| --- | --- | --- |
| Workspace | `workspace_id` | Scope and aggregate Work progress. |
| Work | `{workspace_id, work_id}` plus durable metadata | Outcome, local stage, goal, next action. |
| External issue | source, provider stable ID, display key, URL, external status | Context/reference only; never the Work primary key or board stage. |
| Run | host/socket/session/window execution identity | One current or previous execution attempt for Work. |
| Agent session | daemon agent session attached to a Run pane | Runtime state, model, prompt/response, control target. |
| Signal | attention, blocked, error | Overlay on Work; never a board lane. |

### Work records

Durable records live in `$XDG_DATA_HOME/tend/dashboard-work.json` (normally
`~/.local/share/tend/dashboard-work.json`). Schema v2 keys records by logical
Work identity. Schema v1 host/socket/session/window annotations are migrated
with their old binding retained for compatibility. Writes remain atomic and
serialized by the daemon.

Managed tmux options (`@tend_workspace_id`, `@tend_work_id`, agent role/task)
link a Run to Work. `tend work up` also stores optional external-source metadata
on the window, which `/api/works` discovers and persists. Unmanaged windows stay
in `unlinked_executions`; they are never guessed into Work from their names.
Work only comes from managed runs. `tend work plan WORK --title/--stage/--goal/--next`
edits a Work's metadata (`GET`/`PUT /api/work-metadata`).

## The shell: Home, Explore, Settings

The dashboard's address (`/`) opens Home, as Tend for Mac opens on it.
The activity rail on the left switches between pages:

- **Home**: `/home` (also `/` and `/needs`), described below.
- **Explore**: `/explore`. Every host's panes in the Mac's execution
  topology: host, then session, then window, then pane. Each pane shows its
  agent's mark, title and state, or the shell's command. The panes that need
  you are listed first. The filter in the search field shows All,
  Attention or Active panes, grouped by host tree or by status. Selecting a
  pane of this host shows its live output (`GET
  /api/panes/{pane}/output/stream`) and a field to type to it (Enter sends,
  Shift+Enter adds a line, Interrupt sends Ctrl-C). Both need operator
  access. A pane on another Fleet host is listed with its state and opens
  on that host. `#pane=<id>&socket=<socket>` links to a pane.
- **Settings**: `/settings`. Who is signed in and what this dashboard
  allows. It also opens access administration (operators, viewers, paired
  devices) and pane sharing.

### Old links

The classic workbench was retired in wave 9
([WEB_HOME.md](proposals/WEB_HOME.md#wave-9-retiring-workbench)).
`/workbench` and `/` serve the shell, and its link adapter
(`legacy-link.mjs`) opens what the workbench's links named. A credential in
the link (`#token=`, `?token=`) is stored first, then scrubbed; it is never
copied into the new address.

| Old fragment | Opens |
| --- | --- |
| `pane=<id>` (+ `socket=`) | That pane in Explore. A pane wins over `session`/`window` in the same link. |
| `window=<@id or session:index>` (+ `session=`, `socket=`) | That window's detail in Explore, or its session's when the window is gone. |
| `session=<name>` (+ `socket=`) | That session's detail in Explore. |
| `workspace=<name>` (+ `wsocket=`) | Explore filtered to that workspace's session; Explore says so when no live session matches. |
| `message=<id>`, `thread=<key>` | Explore → Messages. |
| anything else at `/workbench` | Home. |

A visitor whose reads answer 401 is sent to `/welcome` with the mapped
address, without the credential, as where to come back to.

What the workbench had that the shell does not: the Work metadata editor
(use `tend work plan`), Abort all agents of a Work (`POST
/api/work-control/abort` stays for clients; Explore interrupts one pane),
the timeline (`tend timeline`, `tend stats --timeline`; `/api/timeline`
stays) and the Terminals tab (`tend proc run`/`attach`/`detach`;
`/api/terminal-sessions` stays).

## Home (needs you)

`/home` (also `/needs`) is Tend for Mac's Home in the browser: the same
shell (an activity rail with Home and Explore, the Home tab, a status bar
with the tendd chip and the host and agent counts), the same header (Filter
by host or project, Hosts n/m with each host's state, Start Work), and the
same three lanes side by side: **Your turn**, **In progress** and
**Finished**, with the Mac's words, cards, colours and type. The colours,
type and spacing come from one file shared with the Mac app,
`apps/shared/Design/tokens.json` (see
[proposals/WEB_HOME.md](proposals/WEB_HOME.md)). The Windows app opens this
page in its window. On a phone (under 760 px) a segmented control shows one
lane at a time and a unit's details open as a full-screen sheet. The page
follows the browser's language (English or Korean) and colour scheme.

Like the Mac app, Home has two card layouts, switched with the two-icon
toggle in the header and remembered per browser: **status emphasis** (the
default: the lane's colour stripe, a bold title, the question or the latest
line in an inset box, the state, agent, host, project and time on one line;
Your turn cards tinted) and **compact list** (one row per item: a status
dot, the title, the host and how long ago; selecting a row opens its
details, a decision's whole card included).

Units are grouped by project inside each lane, a project placed where its
first unit is, tend's own asks last. **In progress** lists every host's
agents that are working or starting, newest first; **Finished** lists the
agents that went idle in the last 24 hours after doing something, with the
first line of what they last said, and what was decided (below); older
items fold behind "N older". Both read `GET /api/fleet`, so they cover the
Fleet hosts the same way the queue does. An agent is one unit with one
stage, the way the Mac app draws an agent no Work owns; grouping a Work's
agents into one unit, helper folding and the Backlog lane come with a
daemon-side board read (WEB_HOME.md, phase 2).

Selecting a unit opens its details beside the lanes (over them in a narrow
window): what it is doing now or its last result, a field to answer it or
send it the next instruction (`POST /api/panes/{pane}/prompt`, operators
only, local panes only), **Open Pane** (the pane in Explore), its stages
and its agents. Escape closes it.

**Your turn** has everything waiting on you, in the order
`tend review queue` uses: questions agents asked you
(`tend_request_attention`), agents waiting in their pane, then results filed
for review. Each card shows the summary, the decision asked for, the options,
the agent's recommendation and its evidence (commit, PR link, diff, test
result with its output tail, path). A result on a Work with declared checks
shows what tend ran itself first (`verification`: each check, passed or how
it failed, with its output tail), above the agent's own account; tend's copy
of those checks in the evidence is not repeated. The **Finished** tab lists past decisions
with their notes and answers. The page reads `GET /api/decisions` and
`GET /api/decisions/history` (both take `?limit=`, at most 500), the same
daemon reads as the Mac app (`decision_queue`, `decision_history`).

Authorization: both reads are **operator-only**, like pane output. They need
the dashboard token or a live operator sign-in; a viewer session gets 403,
an anonymous visitor 401, and `public_read` or `auth = "none"` never expose
them. They are GETs, so they need no CSRF proof.

The queue is per host (`hosts[]`, each with `alias`, `covers`,
`unavailable` and its `queue`). The host whose tendd serves the dashboard
covers everything. A Fleet host whose tend answers the Fleet
`decision_queue` read (`fleet_decision_queue_v1`) sends its whole queue,
read there as its operator console: questions, waiting agents and review
items. An older Fleet host covers the questions in its operator mailbox and
its waiting agents (from the Fleet snapshot); that mailbox read carries only
the newest messages, so when it is full the section says older questions
may be missing. A host that is not online is listed with its state.

A question on a **control-mode** Fleet host can be answered from the page:
tend reads the request back from that host and answers only if it is
addressed to the operator console (otherwise a Fleet reply would be sent as
the agent in that pane). A review item on a control-mode Fleet host whose
tend applies decisions over Fleet (`fleet_review_dispose_v1`; the section's
`decides_reviews`) can be decided from the page too: that host's relay
checks the option against its own copy of the item, records the decision as
you (`web:<email>`) with the same note as here, and reports what its store
then holds; an answer about any other item is refused. Observe-only hosts'
items, and review items on an older host, are shown but decided on their
own host. Only this route sends that Fleet operation: `fleet_command` over
IPC and `POST /api/fleet/{host}/command` refuse it, so nothing else can name
a person as having decided. The decided half of **Finished** is
per host too: a Fleet host whose tend answers the Fleet `decision_history`
read (`fleet_decision_history_v1`) sends its past decisions, read there as
its operator console; an older or offline host is listed with what kept it
out.

Each card can be decided from the page, through the same daemon paths as
the Mac app:

- a question (human request): an option fills the answer, **Answer…** (or
  **Other…**) opens it, then **Send**; **Decline…** is in the card's `…`
  menu — `POST /api/decisions/requests/{id}`
  with `{"answer": "…", "declined": false, "host": "…"}` (`host` only for a
  question on another Fleet host). Only requests addressed to the operator
  console can be answered, once.
- a result to review: **Accept**, or an option or **Return…** to open the
  note, then **Accept** or **Return** (needs a note); **Withdraw** is in the
  card's `…` menu — `POST /api/decisions/reviews/{id}` with
  `{"disposition": "accepted" | "returned" | "withdrawn", "note": "…",
  "option": "…", "host": "…"}` (`host` only for an item on another Fleet
  host); a chosen option is recorded in the note. Dispositions are
  final.

Both are control routes: the token, or an operator session with the CSRF
proof (`Origin` = the public URL and `X-Tend-Operator: 1`); a viewer gets
403. A repeated identical request changes nothing; a different decision on
something already decided (another disposition, option or note) is a 409. The review disposition records
`web:<email>` (or `web:token`) as its actor, and every web decision is
written as one JSON line, with the signed-in account's issuer and subject,
to `$XDG_DATA_HOME/tend/dashboard-decisions.jsonl`
(owner-only, rotated at 4 MiB) and logged. No dashboard page or static asset can
be framed by another site (`frame-ancestors 'none'`, `X-Frame-Options: DENY`).
Home itself runs under a strict Content Security Policy (its own scripts,
styles and images only; no inline code; `connect-src 'self'`).

The page takes the token from a `#token=…` link (it moves into the
browser's storage and out of the address bar), from the browser's storage,
or not at all when a proxy adds it (the Windows app does). A 403 on a read
other than the queue (`/api/fleet`, the history, `/api/access`), as a paired
device or the Windows app's proxy gets, leaves that part empty; a refused
queue or any 401 drops everything private on the page and offers the
sign-in, which returns to `/home#item=…`.

Notifications link here. With `[sinks.webhook]` enabled, each message ends
with a link to the agent's card (`<dashboard>/needs#item=<session>`) and the
generic payload carries it as `link`. The address is `[sinks.webhook]
dashboard_url`, else `[dashboard.login] public_url`'s origin; set
`dashboard_url = ""` for no link. Opening it still needs an operator
sign-in.

Tend for Mac links here too. Its Home lists the open review items of other
Fleet hosts it can read (control mode, `fleet_decision_queue_v1` and
`fleet_review_dispose_v1`). It does not decide them itself: each card's
**Decide on the Dashboard** opens `<dashboard>/needs#item=<id>&host=<alias>`
(a review id is per host), and the person decides it there, under the
dashboard's sign-in. tendd names the address on `hello` (`dashboard_url`):
`[sinks.webhook] dashboard_url`, else `[dashboard.login] public_url`'s
origin, else this dashboard's own address once it is listening (the port it
got; loopback for `0.0.0.0`), and only when it has a token, since without
one it takes no decisions. With no dashboard, the card says the item is
decided on its host. Home reads at most 100 entries of a host's queue, the
dashboard's own default, so it never links to an item the page leaves out. On the
board, another host's review is keyed `review:<alias>:<id>`, so the same id
on two hosts is two items.

When tendd serves the board read below, Home draws it:
- Work units with their stages;
- helpers folded into the unit they help, with folds for possible helpers, empty sessions and older items;
- the Backlog column, with **Start**;
- **Looks Good** on a finished unit;
- **Review** in the header: the finished results a person asked for and nobody marked yet, over Today, 24 hours, This week or All.

Without the read (an older tendd, or a proxy or paired device it is refused to), the page lays out the queue and the Fleet snapshot itself, one agent per unit.

### The board read (`GET /api/home`)

Home's lanes are computed in tendd, once for every client
(`crates/tend/src/home_board.rs`), by the rules Tend for Mac's Home uses
(`FleetBoard`):

- **Units.** A pipeline run's agents form one Work unit, with one stage per
  agent the pipeline asks for and the run's alias states. Agents in a pane
  stamped with a Work no run lists form one unit per Work. Any other agent
  is a unit of its own.
- **Helpers.** An agent that took another agent's request (a claimed peer
  request, or a `[tend:req_…]` delivery) folds into the unit of the agent
  it helps. A helper whose requester is gone is marked `orphan_helper`. A
  session that only typed a request id is `probable_helper`. A session with
  no prompt, response or recap is `empty_session`.
- **Titles and lines.** The AI title, else the prompt read as a title, for a
  solo agent; the Work id for a Work. The headline is the question when the
  unit needs you, else the recap or the last response.
- **Lanes.** Your turn when any stage needs you, Finished when every stage
  is done, In progress otherwise. A finished unit with a working helper is
  in progress; any unit with a helper that needs you is in Your turn.
- **Usage caps.** A cap on this host's agent that an enabled
  `rate_limited` rule (`swap_account` or `send_prompt`) covers keeps it in
  progress, as `handled_limit`. Any other cap counts as needing you.
- **Your turn order.** The decision queue (the same sections
  `/api/decisions` returns) is interleaved with the units: longest wait
  first, tend's own asks last.
- **Grouping.** Each lane is grouped by project, a project placed where its
  first item is.
- **Finished units** carry `recent` (changed in the last 24 hours),
  `reviewed`, `review_key` and `requested_by_person`. `requested_by_person`
  is what counts toward Review. A finished unit still to review (asked for by
  a person, not reviewed) also carries `review`: `asked`, a digest of the
  prompt (600 characters, Markdown marks dropped, as the Mac's review shows
  it), and `result`, the agent's last response or else its recap, cut to
  4,000 characters with `result_truncated` set when it was cut.
- **Backlog.** Queued Work that is not running is listed in `backlog`.

Words a client owns (state labels, the fixed headlines such as "Waiting on
claude · mini: …") come back as `{"key": …, …}` objects, so each client
says them in its own language. The read is operator-only, like the decision
queue: a viewer gets 403, an anonymous visitor 401. A paired device does not
reach it.

`POST /api/home/reviewed` with `{"id": "<unit id>", "review_key": "<the
unit's review_key>"}` is **Looks Good** on a finished unit. The key names
the result the person saw: if the unit changed since (another turn
finished), tendd answers 409 and records nothing, so a mark never accepts a
result nobody looked at. It records the same review mark Tend for Mac records (an
accepted review whose idempotency key is `home:` plus the SHA-256 of the
unit's id and revision), so a unit marked on the web is marked on the Mac
too, and shows again once it changes. It is a control write (the token, or
an operator session with the CSRF proof), audited like the decisions.

`POST /api/home/instruction` records what the person typed to a unit's
agent from Home, after the words went to its pane (`POST
/api/panes/{pane}/prompt`): `{"id": "<unit id>", "session_id": "<the
agent's session>", "state_entered_at": "<when it entered the state the
words answer>", "kind": "answer" | "send_back", "text": "…", "question":
"<what it asked, optional>"}`. The session must be one of the unit's agents
on the board tendd builds now (404 otherwise). It records the decision Tend
for Mac records: a review on the unit's Work (else that agent's session),
claimed and `returned` in one step, the words as its note, the question as
its summary, under the Mac's key (`home-answer:` or `home-sendback:` plus
the SHA-256 of the session, the wait and the words), so a retry of the same
send records once. It only records; it never sends. Same gate and audit as
Looks Good. The unit is looked up in every lane, so one that the words just
set to work is still found; one that left the board (its agent gone, or
regrouped into another unit) answers 404, and the words stay sent without a
record. A page never sends again because a record failed.

## Paired devices

A phone (the Tend app for iPhone and iPad) does not use the dashboard
token or an operator sign-in. It gets **its own credential**, with one of
two scopes:

- **`decide`** (the default): it reads `GET /api/decisions` and `GET
  /api/decisions/history` and posts the two decision routes above, for
  every host this dashboard covers. Deciding is still real authority: an
  answer can tell an agent to go ahead.
- **`operate`** (opt-in, granted by the approving operator): everything in
  `decide`, plus device-only routes for the app's Home and Hosts, starting
  with `GET /api/device/fleet` (the Fleet snapshot cut down to hosts'
  states and their agents: no SSH targets, labels, capabilities or panes
  without an agent) and `POST /api/device/capture` `{"host": "…",
  "session_id": "…"}` (one agent's recent output: the pane is the one the
  current snapshot lists for that agent session, captured through Fleet
  under `[fleet] capture_policy`; sanitized text only, the last 400 lines
  and at most 64 KiB, one capture per device at a time, each attempt an
  audit line without the output), and `GET /api/device/home`, Home's
  board (`/api/home`, below) with only the fields the app shows: no pane
  ids or sockets in its agent references (a capture names the agent by
  host and session), decisions exactly as the queue lists them, units as
  `"kind": "unit"`, what was decided in `decided` (with each host's
  `truncated`), and the planned Work in `backlog` (title, line, priority,
  pipeline, project; no ticket reference). A device only shows the
  backlog: starting a planned Work names an existing Work, which only the
  operator routes do. `GET /api/device` lists what the dashboard serves the
  device in `features` (`home` always, `capture` and `work` when Fleet
  runs). It also serves
  starting new work, which runs agents with permissions bypassed:
  `GET /api/device/work-options?host=…` (whether `allow_work_start` is on,
  the host's pipelines by name, and the project folders its agents already
  work in) and `POST /api/device/work` `{"idempotency_key": "<uuid>",
  "host", "project", "pipeline", "request"}`:
  - **New work only.** The dashboard mints the Work id (`phone-MMDD-…`) and
    launches with `new_only`, which tendd refuses for an id that exists.
    The device cannot name an existing Work.
  - **Nothing else to choose.** The project must be one listed for that
    host; the pipeline one it has; no workspace, skill, context, ticket or
    dry run. A `[[route]]` that would match the minted id and set its own
    `cwd`, worktree or `prepare` refuses the start. The host must be this
    one or an online control-mode Fleet host, and `[dashboard]
    allow_work_start` must be on (else 501).
  - **Once per key, durably.** The start is reserved in the device store
    before anything runs. A retry with the same key returns the first
    start's state; the same key for another request is 409. A start that
    timed out, or was reserved when tendd stopped, is `unknown` and is never
    run again: check the host. One start per device at a time, at most 20 an
    hour.
  - **Audited before and after.** An intent line (device, approver, key,
    fingerprint, host, project, pipeline, minted id, request length, not
    its text) is written and synced before dispatch; if it cannot be,
    nothing starts (503). A result line follows.
  `started` means the Run was registered; its agents launch shortly after,
  as for any `tend work up`.

Every operator route refuses a device token of either scope as it would
refuse no credential at all: no `/api/fleet`, pane output or input, Fleet
commands, Work control, shares, access management or new pairings. A
device route outside its scope answers 403.

Pairing is shaped like the OAuth device authorization grant (RFC 8628),
with tend's own endpoints: no secret passes through a link, the clipboard
or another app.

1. The app sends `POST /auth/devices/request` `{"name": "…", "platform":
   "ios"}` and gets `{device_code, user_code, expires_in, interval}`. It
   keeps the device code and shows the user code (`BCDF-GHJK`).
   It may add `"scope": "operate"` to ask for hosts and new work; anything
   else asks for `decide`.
2. An operator opens **Manage access…** in Settings and types the code
   under **Devices**. The dashboard first looks it up without granting
   anything (`POST /api/devices/lookup` `{"user_code": "…"}`: the name,
   platform, requested scope and when it asked), then the operator approves
   that request (`POST /api/devices/approve` `{"user_code": "…", "scope":
   "decide" | "operate"}`). For a device that asked for `operate`, the
   dialog shows a separate, unticked box; without the tick it is paired to
   `decide`. The device gets the lesser of what it asked and what was
   granted; a scope left out is `decide`. Approve only a code shown on a
   device you are holding, right now: a code someone sends you would pair
   *their* device (RFC 8628, section 5.4), and with `operate` it could
   start agents on your hosts.
3. The app polls `POST /auth/devices/token` `{"device_code": "…"}` every
   `interval` seconds: `202 {"status": "pending"}` until approved, then
   `201 {"device": {id, name, platform}, "token": "tdv_…", "scope": [...]}`
   exactly once (`["decisions"]` or `["decisions", "operate"]`, as `GET
   /api/device` also says).
   It sends `Authorization: Bearer tdv_…` from then on.

Limits and lifetime:

- A request lives 10 minutes; at most 32 are pending (then 429). The
  device code and the token are 244 random bits; the user code is 8
  characters and only a signed-in operator can submit it.
- A device stops working after 30 days unused or 90 days after pairing,
  whichever comes first, and the moment it is removed (**Remove**, **Remove
  all**, or the app's Sign out, `POST /api/device/forget`). At most 16 are
  paired.
- **Limit to deciding** (`POST /api/devices/{id}/limit`) drops an
  `operate` device to `decide` at once, keeping it paired. It is written to
  the store before it answers and cannot be undone for that token; to widen
  a device, pair it again. Neither limiting nor removing undoes an agent a
  device already started.
- Every use counts towards the idle limit, so a token in daily use, by its
  owner or by whoever stole it, lasts the full 90 days: removing it is the
  defence.
- Devices are their own principals. Removing the operator who approved a
  device, or rotating the dashboard token, does **not** unpair it: use
  **Remove all**.
- Pairing needs a configured dashboard token (it is what makes anyone an
  operator); without one the routes answer 404 and decisions 403.

A device decision is recorded as `app:<name>#<id prefix>`. Its line in
`dashboard-decisions.jsonl` names the device id and who approved it; pairing,
approval and removal are written there too, never with a code or a token.
A decision on another Fleet host carries only the label to that host; the
full attribution is in this host's audit file.

Paired devices are stored in SQLite at
`$XDG_DATA_HOME/tend/dashboard-devices/devices.sqlite3` (mode `0600` in a
`0700` directory, exclusive lock, refused if it is not a tend device store):
the device id, name, platform, who approved it, the times, and the SHA-256
of its token, never the token. The file is created when the first device
pairs. `/api/access` reports `capabilities.device_pairing`.

## Collaboration graph and history API

Explore → Messages (`/explore/messages`) projects retained requests into two linked views. The
graph uses stable host/socket/agent-session nodes (plus one operator-console
node) and aggregates directed request edges by peer pair, count, kind, and
status; replies appear as dashed reverse edges. Selecting an edge or room
drills the adjacent sequence into chronological request/reply arrows. Selecting
an event opens its thread/parent, Work/Run, artifact, link, message, and
reply details when the response is authorized. The sequence caps the rendered
tail at 200 requests, while **load more** extends the retained result used by
the graph and drill-down.

The toolbar filters by time range, exact room, Work, thread, kind, and status.
The API form is:

```text
GET /api/collaboration?since=7d&session=webapp&socket=%2Ftmp%2Frmux%2Fdefault&workspace=webapp&work=ENG-7345&thread=thread-id&parent=req-id&kind=review&status=completed&limit=100&cursor=...
```

All supplied filters are conjunctive. `session` matches the durable tmux
session name (including its numeric `~view~` aliases); `socket` pins its exact
server endpoint. These filters apply before counts and pagination. `work_id`, `workspace_id`, `thread_id`,
and `parent_request_id` are accepted as aliases for their short names. `room`
is a URL-encoded JSON `RoomId` and matches host, socket, and window exactly.
`limit` defaults to 100 and must be 1–500. Pagination is newest-first keyset
pagination: pass opaque `pagination.next_cursor` back as `cursor`; do not parse
it or replace it with an offset. The response contains `generated_at`,
`details_included`, `requests`, and
`pagination { total, limit, has_more, next_cursor }`. Invalid filters return
`400`; a disabled collaboration subsystem returns `503`.

When `details_included` is false, request/reply/update bodies, provenance, paths,
generic artifacts, and links are omitted. Participant, room,
thread/parent, Work/Run, kind, status, timestamps, and counts remain available
for a useful summary graph. In Explore, a session's or pane's messages link
filters the history to it. Use **Upgrade with token** (Settings) or sign in as
an operator to read requests, replies, and artifacts.

Messages opens as a message list beside a detail reader. The list shows
participants, status, timestamps, response latency and update counts. Search
filters retained history on the server before pagination. Public and viewer
searches cover visible metadata; authenticated operators can also search
request, reply, and progress bodies. Counts reflect all matching rows, while
summaries of loaded pages are labeled when older pages remain. Custom period
controls accept a start and end time. Graph and Sequence are separate views;
the graph supports zoom and fit in both dimensions. Thread and parent buttons
open related history across rooms, and message links
(`/explore/messages#message=<id>`) reopen an exact retained request even
outside the default period. Copy-link strips all other URL parameters.

## Fleet hosts

When `[fleet] enabled = true` and `[fleet.hosts]` lists SSH hosts, Home and
Explore cover every host: Home's header shows Hosts n/m with each host's
state, and Explore lists each host's sessions, windows and panes (this node
first; an offline host shows the relay's error). Both read `GET /api/fleet`,
which follows the same read policy as `/api/agents`; it is read-only, and
`POST /api/fleet/{host}/command` is not wired to it.

## Sessions, windows and panes

tend tracks every agent by where it runs: a tmux (or rmux) **session**, a
**window** in it, and a **pane** in that window. Explore lists them the same
way, under each Fleet host: host, then session, then window, then pane, from
`GET /api/fleet` (each host's `/api/panes` joined with its agents). Selecting
a session or a window opens its detail: its panes, with their state and
latest activity, as Tend for Mac's session and window detail draw them.
`#session=<name>`, `#window=<@id>&session=<name>` and `#pane=<id>` (with
`&socket=<socket>` when more than one tmux server is running) link to them.

`GET /api/windows/{window}/layout` (tmux's own pane geometry) is metadata,
so viewers and `public_read` visitors can read it; the socket is looked up in
the scanned inventory, never used as a caller-supplied path. Viewers see the
tree and the details but no pane output and no composer.

## Talking to an agent

Select a pane in Explore (or **Open Pane** on a Home unit) to open it. The
pane view shows its agent, state and working directory, and below that the
pane's live output, streamed while the pane is open and the tab is visible.
Type in the field and press Enter to send (Shift+Enter adds a line; Enter
that confirms an IME candidate never sends). **Interrupt** sends Ctrl-C.
`#pane=<id>` (plus `&socket=<socket>` when the id exists on several tmux
servers) links straight to it.

Output and sending are for operators (the token, or an operator sign-in).
Pane output can contain anything the agent printed, including secrets, so
`GET /api/panes/{pane}/output` is gated like a write even though it is a GET:
a viewer gets 403, and in `public_read` an anonymous visitor needs the token.
Viewers and `public_read` visitors can still select a pane, but Explore shows
only its metadata and never requests output. With `auth = "none"` nobody
can read output. The live stream (below) is gated the same way.

### Live pane output

Explore's pane view reads
`GET /api/panes/{pane}/output/stream?socket=&lines=N`, a server-sent event
stream with the same access rules, validation and `Cache-Control: no-store` as
`/output`.

- **One capture loop per pane.** Every viewer of a pane (every tab, every
  client) shares one loop in the daemon. It runs only while someone is
  connected: it captures the pane every 300 ms, strips escapes with the same
  sanitizer as `/output`, and publishes only when the text changed. It stops
  within one tick after the last viewer leaves. A capture still running when
  the next tick comes skips that tick, so tmux calls never pile up.
- **Events.** `event: output` carries `{ pane, text, captured_at }` with the
  whole bounded text (newest `N` lines, 64 KiB cap), first as a snapshot on
  connect and then on every change to that viewer's lines; an unchanged pane
  sends nothing but a keep-alive comment every 15 seconds. `event: gone`
  (`{ pane }`) means two captures in a row failed, usually because the pane
  closed; it is the last event.
- **Limits.** At most 32 panes stream at once across all clients, and 8
  viewers per pane; beyond that the request gets `429`. A stream closes after
  30 minutes and the client reconnects, so a revoked credential cannot hold a
  stream open.
- **Client.** The dashboard reads the stream with `fetch()` (like
  `/api/events`), so bearer tokens and sign-in cookies both work. It
  reconnects with exponential backoff (0.5 s doubling to 15 s, jittered) and
  closes the stream while the tab is hidden or the pane is closed. Against an
  older daemon that answers `404`, it falls back to polling `/output` every
  1.5 s.

### Nerd Font glyphs

Shell prompts (powerlevel10k, starship) and agent status lines often print
Nerd Font icons, which live in Unicode's Private Use Areas. Explore's pane
output and the shared-pane page use a bundled subset of **Symbols Nerd Font Mono**
and lists it after the primary monospace font with a `unicode-range` of
U+E000–U+F8FF and U+F0000–U+FFFFF, so ordinary text keeps your system's
monospace font and browsers fetch the icon font (about 1.2 MB, served from
`/static/fonts/`) only when output actually contains such a glyph. Both
pages' Content-Security-Policy allows it with `font-src 'self'`.
See [Third-party notices](#third-party-notices).

## Starting work from the board

`POST /api/work-control/up`, and Home's **Start Work**, create a Work's
team: give it a stable Work id and,
optionally, a separate external issue key. Tend routes it to a workspace and
directory and creates whichever agent panes the pipeline declares but the Run
does not have yet.
Pressing it twice fills gaps rather than duplicating the team; with a `body` it
also prompts the agents already running. See [PIPELINE.md](PIPELINE.md) for the
`[[route]]` and `[pipeline.*]` configuration it needs.

It is off until you set `[dashboard] allow_work_start = true`, **on top of** the
control token. Every other write route steers a process you already started;
this one starts new ones with permissions bypassed. That is a different kind of
authority, so it gets its own grant — the way `[ask]` and `[collaboration]` do.
Disabled, the route answers `501` rather than `403`, because the token is fine;
the capability simply is not offered here.

The daemon runs the `tend` binary for this rather than reimplementing the
pipeline. The launcher and the managed-tmux registry live in the CLI crate,
which depends on the library the daemon links, so calling them in-process would
mean inverting that dependency or keeping a second implementation alive.

An external issue title and provider status stay in `external_items`; they do
not overwrite the local Work title or stage. The response reports
`linked_external_item`, while the compatibility field `seeded_metadata`
remains false. This keeps provider sync and operator-authored workflow state
independent.

## Timeline

`GET /api/timeline` serves the same intervals as `tend timeline` and
`tend stats --timeline` (the browser timeline went with the workbench). It
defaults to `since=7d` and also takes `24h`, `today`, `last week`, `month`,
`last month`, `30d`, `12w` or a `YYYY-MM-DD` day, `session=<name>` and
`agent=<kind>` (`codex`, `claude_code`, `antigravity`, `opencode`,
`unknown`), and comma-separated, case-sensitive `exclude-pane` /
`exclude-session` globs, for example
`/api/timeline?since=month&exclude-session=monitor*`. Overlapping agent
intervals are unioned into rows for working, waiting, error, idle and
starting; interaction and tmux presence have their own rows, and numeric
client-view sessions are folded into their base session.

The `act` metric follows `tend stats ACT`: engaged human time estimated from
submitted prompts, tmux input ticks, and time spent present while agents are
waiting for a human answer. The `human` metric follows `tend stats HUMAN`:
the union of tmux foreground time and tend-recorded human interaction
intervals.

Closed intervals come from `activity.ndjson`. Currently-open agent states
come from the live `Store` snapshot, and currently-open tmux foreground spans
come from `session-activity.json` when that tracker is enabled.

## Live updates

`/api/events` is a fetch-streamed SSE connection for clients that follow
agent state. Three event types appear on the wire:

- **`snapshot`** — sent once on connect. Payload: `{ "agents": [...] }`.
  Lets a freshly-loaded page paint without a `/api/agents` round-trip.
- **`transition`** — emitted on every state change. Payload: a serialized
  `Transition`. The client mutates one row in place.
- **`lagged`** — when the broadcast receiver falls behind the server's ring
  buffer. The client refetches `/api/agents` for a clean baseline.

(`EventSource` can't carry `Authorization` headers, so a browser client uses
`fetch()` and parses SSE frames itself, as `pane-stream.mjs` does.)

Pane output has its own per-pane stream; see
[Live pane output](#live-pane-output).

## Why the HTML is public

The static HTML/JS/CSS routes intentionally sit *outside* the auth middleware.
Three reasons:

1. **Bootstrap.** Browsers can't inject custom headers on top-level
   navigation. The first GET has to succeed unauthenticated for the JS to
   start, read `#token=...`, and persist it in `localStorage`.
2. **No embedded data.** The bundle holds no agent state. It's a thin client
   that asks `/api/*` for everything. API access follows the selected auth
   mode.
3. **The cost is bounded.** An unauthenticated GET to `/` returns the same
   bytes for everyone. There's nothing to leak.

If this tradeoff doesn't work for you, front the daemon with a reverse proxy
that strips the carve-out (or use mTLS).

## Scope and limits

- External issue references are captured at `tend work up` time. Continuous
  two-way Linear/GitHub/Jira synchronization, dependencies, and estimates are
  not implemented.
- Control is intentionally narrow: Work/pane prompt and abort plus
  Tend-owned PTY input/terminate. There are no direct configuration/file editing
  APIs. Sending a prompt to a shell pane can execute shell commands.
- No mobile UI. The CSS scales OK to ~600 px but isn't designed for phones.
- Operator access uses a bearer token, or optionally
  [OIDC sign-in](#operator-sign-in-with-oidc) restricted to one group. Invited
  recipients use OIDC with access limited to their shared panes; they cannot
  manage the dashboard.

The [`dashboard::router`](../crates/tend/src/dashboard/server.rs) function is
public for embedding and integration tests.

### Third-party notices

The dashboard embeds `tend-nerd-symbols.woff2`, a subset of Symbols Nerd Font
Mono from [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts) v3.5.1 (MIT,
copyright Ryan L McIntyre). The icon sets it contains keep their own licenses
(Font Awesome and Codicons CC BY 4.0, Material Design Apache 2.0, Pomicons and
Weather Icons SIL OFL 1.1, the rest MIT). The license text, source URL,
checksums and the command that produced the subset are in
[`crates/tend/src/dashboard/web/fonts/`](../crates/tend/src/dashboard/web/fonts/README.md);
the daemon also serves the license at `/static/fonts/LICENSE-nerd-fonts-symbols.txt`.

<a id="operator-sign-in-with-oidc"></a>

## Operator sign-in with OIDC

Instead of pasting the bearer token into every browser, the dashboard owner can
sign in through any standards-compliant OpenID Connect provider (Keycloak,
Authentik, Dex, and similar). Sign-in is deny by default; each verified account
gets exactly one role, checked in this order:

| Role | Granted when | Can |
| ---- | ------------ | --- |
| operator | the ID token's group claim contains `required_group`, or the account was **enrolled** with the dashboard token ([Enrolling an account with the token](#enrolling-an-account-with-the-token)) | everything the bearer token can |
| viewer | the group claim contains `viewer_group`, or the account matches a [viewer rule](#viewers) | read the dashboard and its event stream, redacted exactly like an anonymous `public_read` visitor; no writes, no shares, no access management |
| none | nothing above | nothing: no session is created. With enrollment on it gets the token page, otherwise a refusal page |

Pane and window share recipients (`/share/*`) are a separate feature with their
own sessions: a viewer session is not a share session, and neither implies the
other.

Signing in grants the same authority as the bearer token, so restrict the group
to the people who should operate this machine's agents. Enrollment lets the
token holder add their own account without editing the provider's groups.

```toml
[dashboard]
enabled = true
bind = "127.0.0.1:7878"
auth = "token"
# Keep the token (dashboard.token or TEND_DASHBOARD_TOKEN). It remains the
# API/CLI credential and the way in when the provider is unavailable.

[dashboard.login]
public_url = "https://tend.example.com"      # origin only; HTTP only on loopback
issuer_url = "https://login.example.com/realms/example"
client_id = "tend-dashboard"
# required_group = "tend-operators"          # optional while enrollment is on
# enrollment = true                          # default; false = group-only sign-in
# viewer_group = "tend-viewers"              # optional read-only group
# viewer_rules = ["email:*@example.com"]     # optional read-only viewer rules
# groups_claim = "groups"                    # default "groups"
# scopes = ["groups"]                        # requested in addition to "openid email"
# client_secret_env = "TEND_LOGIN_CLIENT_SECRET"  # confidential clients: env var NAME
```

Provider requirements:

- Discovery at `{issuer_url}/.well-known/openid-configuration` with HTTPS
  authorization and token endpoints.
- Authorization code flow with **PKCE S256**. A public client
  (`token_endpoint_auth_method = none`) works; a confidential client uses
  `client_secret_env`.
- The exact redirect URI `https://tend.example.com/auth/callback`.
- A signed ID token whose `groups_claim` holds the group names, as a JSON array
  of strings or a single string. Some providers release it only with an extra
  scope (add it to `scopes`) or a client/claim mapper. `email` is shown in the
  dashboard when present but is not used for authorization and need not be
  verified.

Rules checked at startup: `required_group`, when set, is a non-empty group
name; `required_group` is set when `enrollment = false`; `auth = "token"` with
a token is configured; and when `[dashboard.sharing]` is also set, both
sections use the same `public_url`. Restart the daemon after changes.

Without `required_group`, the provider only proves who someone is: nobody is an
operator until the token holder enrolls them.

Flow and session contract:

| Route | Purpose |
| ----- | ------- |
| `GET /auth/login?return_to=/path` | Starts the flow (PKCE S256, state, nonce, `prompt=select_account`). `return_to` must be a same-origin path beginning with a single `/`. |
| `GET /auth/callback` | Verifies the ID token (signature, issuer, audience, expiry, nonce, `at_hash`), then redirects to `return_to` if the account is an operator, or to `/auth/enroll` if it is not and enrollment is on. |
| `GET /auth/enroll` | Enrollment page for this browser's pending enrollment. |
| `POST /auth/enroll` | JSON `{"token": "..."}`; enrolls this browser's pending account, or its signed-in viewer ("upgrade with token"), and signs it in as an operator. |
| `GET /auth/enroll/cancel` | Discards this browser's pending enrollment. |
| `POST /auth/logout` | Ends the session. |
| `GET /auth/session` | Unauthenticated `{available, signed_in, role, email, via, enrollment, login_url}` so a signed-out page can offer sign-in. `role` is `none`, `viewer` or `operator` (the bearer token counts as `operator`); `via` is `group`, `enrollment`, `viewer_group`, `viewer_rule` or `null`. `/api/access` includes the same object as `login`. |
| `GET /api/operators` | Operator only: `account` (the caller: `{role, via, email, subject}`, `via` is `token` for the bearer token) and enrolled accounts `{id, email, subject, issuer, created_at, last_seen_at, current, active}`. |
| `POST /api/operators/{id}/remove` | Operator only: removes the account and ends its sessions. |
| `GET /api/viewers` | Operator only: `{viewer_group, max, rules: [{id, rule, source, active, created_at}]}`; `source` is `config` (read-only, `id` null) or `dashboard`. |
| `POST /api/viewers` | Operator only: JSON `{"rule": "email:*@example.com"}`; 201, 400 for an invalid rule, 409 for a duplicate or the 128-rule limit. |
| `POST /api/viewers/{id}/remove` | Operator only: removes a dashboard rule and ends viewer sessions it no longer allows. |

- The session cookie is `__Host-tend-op` (`tend-op-dev` over loopback HTTP),
  `HttpOnly; Secure; SameSite=Strict; Path=/`, lasting 8 hours. A short-lived
  `__Host-tend-op-login` cookie (`SameSite=Lax`) binds the pending login to the
  browser that started it.
- Group membership is checked at sign-in. Removing someone from the group takes
  effect when their session expires, they sign out, or the daemon restarts.
  Enrollment is checked on every request, so removing an enrolled account ends
  its sessions immediately. Sessions are in memory only and never written to
  disk.
- Every read route, SSE included, accepts either the bearer token or an
  operator or viewer session cookie. Write and management routes answer a
  viewer session with 403. A **state-changing request authorized by the cookie** must also
  send `Origin: <public_url>` and `X-Tend-Operator: 1`, or it is refused with
  403. The dashboard's own JavaScript does this; bearer-token clients do not need
  to. `POST /auth/logout` has the same requirement.
- Sign-in state is separate from pane sharing: a recipient session never
  authorizes operator APIs, and the operator cookie is not a recipient session.

Behind a reverse proxy or tunnel, preserve the public `Host` header (the
loopback-bound dashboard accepts the configured public host and rejects other
names), forward cookies unchanged, and do not cache `/auth/*` or `/api/*`.
Omit query strings on `/auth/callback` from access logs: they contain the
authorization code. Tend's own logs redact `code`, `state` and `token`.

Opening the dashboard on another origin (for example `http://127.0.0.1:7878`)
still works with the token; its **sign in** button sends the browser to the
public origin, because the callback and cookie belong there.

### Enrolling an account with the token

With `enrollment` on (the default), an account the provider verifies but that is
not an operator is not turned away:

1. The callback holds the verified issuer, subject and email for five minutes in
   a pending enrollment bound to this browser by an `HttpOnly; SameSite=Lax`
   cookie, `__Host-tend-op-enroll` (`tend-op-enroll-dev` over loopback HTTP),
   and redirects to `/auth/enroll`.
2. That page shows the signed-in account and asks for the dashboard's access
   token. Its script sends the token once, as JSON, to `POST /auth/enroll`.
3. With the right token the account is enrolled, the pending cookie is
   cleared, and the browser gets a normal operator session and returns to
   where it started. From then on the account signs in directly.

**Cancel** discards the pending enrollment. A signed-in viewer can enroll its own
account the same way with **Upgrade with token** (Home or Settings); the viewer
session is replaced by an operator session. To remove an enrolled account, open
**Manage access…** in Settings (shown to an operator) and choose **Remove**, or call `POST /api/operators/{id}/remove` with
the bearer token. Removal ends that account's live sessions at once; signing in
again leads back to the enrollment page. Group members are not listed and need
no enrollment.

Security notes:

- Enrollment grants nothing beyond the token: only someone who already holds
  the dashboard token, and so already has full operator rights, can enroll an
  account. A pending enrollment authorizes no API route by itself.
- `POST /auth/enroll` requires the pending cookie, `Origin: <public_url>` and
  `X-Tend-Operator: 1`, and a JSON body. A cross-site page can neither send the
  `Lax` cookie on a POST nor add the header without a CORS preflight the
  dashboard never grants. The page's CSP allows only its own same-origin script
  and blocks native form submission, so the token cannot end up in a URL.
- The token is compared in constant time and never logged. Each pending
  enrollment allows five wrong tokens and is then discarded, so a new provider
  sign-in is needed. Every wrong token also delays the next attempt from any
  browser by one more second (up to 30 seconds). At most 16 enrollments may be
  pending at once.
- Enrollment is keyed on the ID token's issuer and `sub`, never the email, and
  is pinned to the configured `issuer_url`: entries recorded for another issuer
  are listed but never match.
- At most 64 accounts can be enrolled. Pane-sharing recipient sessions cannot
  enroll or reach `/api/operators`.
- The viewer upgrade uses the same `POST /auth/enroll` checks: the `Strict`
  session cookie plus `Origin` and `X-Tend-Operator: 1`, the constant-time
  comparison, the global backoff, and five wrong tokens per viewer session,
  after which that session is ended.
- `enrollment = false` restores group-only sign-in: the refusal page returns,
  and previously enrolled accounts stop being honored. They are kept on disk
  and are honored again if enrollment is turned back on.

### Viewers

Operators manage viewer rules in the **access** dialog. Type an email address
(`ann@example.com`) or a domain pattern (`*@example.com`); the dialog stores
them as `email:ann@example.com` and `email:*@example.com`. The API and
`viewer_rules` also accept `sub:<subject>`.

- Email rules match only when the ID token says `email_verified: true`, and
  compare case-insensitively. A domain rule matches that exact domain:
  `*@example.com` does not match `ann@sub.example.com`. There are no other
  wildcards.
- `sub:` rules match the subject exactly. Every rule is pinned to the configured
  `issuer_url`; rules recorded under another issuer are listed but never match.
- Rules are re-checked on every request, so removing one ends the viewer
  sessions it allowed at once. `viewer_group` is checked at sign-in, like
  `required_group`.
- `viewer_rules` in the configuration are merged with the dashboard's rules and
  shown read-only. At most 128 of each.
- An account that is both an operator and a viewer signs in as an operator.

Enrolled accounts and viewer rules are stored in SQLite at
`$XDG_DATA_HOME/tend/dashboard-operators/operators.sqlite3` (the platform data
directory, next to the pane-sharing store): mode `0600` in a `0700` directory,
held under an exclusive lock so a second daemon cannot open it, and refused if
the file is not a tend operator store. Each row holds the issuer, subject, the
display email, and the enrollment and last sign-in times; viewer rules keep the
rule, its issuer and when it was added. Nothing is opened when sign-in is off.

<a id="invite-someone-to-a-pane"></a>

## Invite someone to a pane or window

Authenticated sharing gives an invited account access to a local **tmux or rmux
pane**, or the current panes in its **window**. Choose the scope when creating
an invitation. A window invitation pins up to 16 pane processes; panes created
later are not included. The recipient selects a pane to view or send commands.
If one pane ends, the other original panes remain accessible. Choose **Share…** on a pane in Explore, enter the recipient's email,
choose **View output** or **View and send prompts**, then copy the generated link.
**Manage shares…** in Settings lists and revokes invitations. The recipient
signs in on a separate page; the operator bearer token must not be sent to them.

Configure an OIDC application that supports authorization code flow with PKCE
and returns `email` and `email_verified = true` in its ID token. Register the exact
redirect URI `https://tend.example.com/share/auth/callback` with that provider.
Example configuration (replace the example host and issuer):

```toml
[dashboard]
enabled = true
bind = "127.0.0.1:7878"
auth = "token"
# Supply the operator token through TEND_DASHBOARD_TOKEN or dashboard.token.

[dashboard.sharing]
public_url = "https://tend.example.com"
issuer_url = "https://login.example.com"
client_id = "tend-dashboard"
# Optional for confidential clients; this is the environment variable NAME.
client_secret_env = "TEND_SHARING_CLIENT_SECRET"
# Optional: absolute path on a private local filesystem.
# storage_path = "/absolute/path/to/shares.sqlite3"
```

Terminate HTTPS at a reverse proxy forwarding to the loopback dashboard, preserve
its public `Host`, and forward `/share/*` and the dashboard static assets. The
configured `public_url` must be an origin without a path, query or fragment.
HTTP is accepted only for loopback development. Sharing requires private
`auth = "token"`; `public_read` and `none` are incompatible. Set the client secret
in the daemon's environment when `client_secret_env` is specified. Restart after
configuration changes. No login provider or public listener is enabled automatically.

Invitations expire after 1 minute to 24 hours (the UI offers 1, 4 or 24 hours).
Up to 256 invitations are retained until expiry, including revoked invitations;
if the limit is reached, wait for previous invitations to expire.
Sessions last at most 8 hours. Invitations, bound identities, revocations and command
receipts are saved in a private SQLite database under the platform data directory
(`tend/dashboard-sharing/shares.sqlite3`). **Daemon restarts preserve invitations;
recipients sign in again.** The pane process must still be the original process.
Session cookies and pending login flows remain in memory and are never saved. The first successful
login binds the verified invited email to the provider's subject identifier.
Other accounts cannot use the link, and recipient cookies grant no operator API
access. Logging out ends that browser session; use **Revoke** to end the invitation.
Revocation waits for an already accepted prompt to finish and rejects subsequent
prompts. It cannot undo commands already executed.

Prompt access submits literal text followed by Enter to the existing process.
A shell pane therefore permits shell commands; share only with someone who should
have that authority and may see the pane's output. This is not a command sandbox.
The socket, session, window, pane and process identity are checked again before
operations; replacing the pane requires a new invitation. Remote panes, terminal resizing and raw keyboard streaming are not included.
Tmux servers with colliding short socket names must be renamed before sharing.

The recipient page polls visible output every 2 seconds and pauses while hidden.
Output capture has a 1-second cache, a 32,768-character limit and a bounded pool
of eight concurrent pane operations. Prompt requests are limited to 16 KiB and
one per 500 ms per share, with at most 1,024 distinct commands per invitation.
All invitations to the same pane serialize command delivery. Failed submissions
are never retried automatically. The page reconnects with bounded backoff after
network failures and disables sending until it has a fresh successful read.

Operator API: `GET/POST /api/shares`, `POST /api/shares/{id}/revoke` (bearer token).
Create body: `{"scope":"pane","pane":"%1","socket":"/path/to/tmux/socket","email":"guest@example.com","permission":"prompt","ttl_seconds":3600}`.
Use `"scope":"window"` to include the current window members.
Recipient API: `GET /share/api/{id}` (optional `?pane=%252` to select `%2`), `POST /share/api/{id}/prompt` with
`{"text":"...","request_id":"a-unique-command-id","pane":"%2"}`.
The pane must belong to the invitation; omitting it selects the original pane. Reuse the same ID, selected pane and exact
text when checking an uncertain delivery. A confirmed receipt returns success
without executing again; an interrupted/uncertain receipt returns 409 and is
never replayed, including after restart. A changed payload under the same ID is
rejected. IDs omitted by legacy callers cannot deduplicate separate requests.
Recipient writes require the session cookie, the configured
origin and `X-Tend-Share: 1`.

### Deployment and recovery

1. Register an OIDC client with authorization code flow, PKCE S256 and the exact
   public callback URI. The ID token must contain a verified email. Keep the
   operator bearer token separate from the OIDC client secret.
2. Configure `dashboard.sharing` and supply the named secret environment variable
   to the daemon service, then restart. `storage_path` may override the database
   location with an absolute file path on a local filesystem. Keep the database
   and its lock file private to the daemon user; tend creates them with mode 0600.
3. Forward the HTTPS origin to the loopback dashboard without rewriting `Host`,
   stripping cookies or caching `/share/*` and `/api/*` responses. Configure the
   proxy access log to omit query strings on `/share/auth/callback`: they contain
   authorization codes. Tend's own HTTP logs redact code, state and operator
   tokens, including percent-encoded parameter names.
4. In **Shares**, choose **Check login setup**. It checks discovery and signing
   key reachability; it cannot validate a client secret or email claim without an
   actual login. Create a short-lived invitation and test the recipient login,
   command, daemon restart and revocation before distributing links.

`GET /api/shares/status` reports whether configuration, private storage and the
client-secret environment variable are present. `POST /api/shares/check` checks
provider discovery. Both require the operator bearer token. Recipients can use
**Sign out all devices** to end their current account's browser sessions without
revoking the invitations themselves.

Tend commits an invitation, account binding or revocation before confirming it.
It also records command acceptance before sending input. If storage fails, the
change is not acknowledged and a new command is not executed. If the process dies
between acceptance and completion, the command's result is uncertain: inspect the
pane, then explicitly start a new command if appropriate. This guarantees that
replaying the same request ID does not intentionally execute it twice; it cannot
make a terminal side effect transactional with the database.

The database is leased to one daemon. A second daemon using the same file is
rejected; use separate storage paths for separate instances. Corrupt data,
unsupported storage versions, or changes to the public origin, issuer or client
ID prevent the sharing dashboard from starting. The error is in daemon logs.
Restore the original configuration or choose a new storage path to begin with no
existing grants. Never silently replace a corrupt database with an empty one.
For backups, stop the daemon before copying the database. Restoring a backup can
restore permissions that were revoked after that backup; review invitations
before reopening external access.

The optional browser smoke test exercises an isolated daemon, test OIDC provider
and private tmux server. It covers login, command delivery, response loss,
reconnection, restart, recipient isolation and revocation. It needs Python's
`cryptography`, Node's `playwright`, and a Chromium executable:

```sh
cargo build --workspace --bins
# Configure these only if playwright/Chromium are outside their default locations.
export TEND_PLAYWRIGHT_PACKAGE=/path/to/playwright
export TEND_TEST_CHROMIUM=/path/to/chromium
python3 scripts/dashboard-sharing-smoke.py
```

On Windows the harness runs the panes on a private rmux label instead
(`TEND_TEST_RMUX_LABEL`, default `tend-share-smoke`) and needs PowerShell
and Python on `PATH`; build with `cargo build -p tendd` and point
`TEND_TEST_CHROMIUM` at an installed Chrome.

This harness creates only loopback listeners and fixture identities. Passing it
does not replace the deployment's own HTTPS and real-provider login check.

### OIDC dependency review

`openidconnect` currently brings in `rsa` for ID-token signature verification.
The dependency scanner exception for [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)
is limited to that advisory: its private-key timing disclosure does not apply to
Tend's public-key verification path. Tend does not import RSA private keys, sign
RSA client assertions or decrypt RSA messages. The upstream advisory has no
patched version as of this review (2026-09-27). Reassess this exception before
adding any private-key RSA operation or changing the OIDC dependency.
