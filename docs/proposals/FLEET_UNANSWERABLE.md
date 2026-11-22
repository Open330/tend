# Keep and Close all across the Fleet

Status: approved 2026-11-19; being built in three PRs (1: relay and
manager, #667; 2: dashboard routes; 3: web, iOS and the Mac hand-off). Follow-up to
[STALE_QUESTIONS.md](STALE_QUESTIONS.md).

## Today

Each host folds its own unanswerable questions, and the person can Keep one
or Close all only on the host that holds them:

- The local section of `GET /api/decisions` carries `subject_check: true`
  and `close_unanswerable`, and the Keep and Close all routes act on the
  local collaboration store only.
- A remote section arrives through the `DecisionQueue` relay operation. Its
  items already carry `unanswerable` when the remote host is new enough, so
  the folded group can be drawn, but the section has no `subject_check` or
  `close_unanswerable` flag. The surfaces therefore show no Keep or Close
  all for it.
- Review decisions already cross the Fleet: `ReviewDispose` with
  `fleet_review_dispose_v1`, control-mode hosts only, gated in
  `fleet_manager.rs` (`review_dispose_refusal`).

## Proposal

Mirror `ReviewDispose`: the decision is applied by the host that holds the
item, under that host's own rules and locks.

### Relay operations

Two new controller operations, both `is_control()`, and their relay
envelopes. As for `ReviewDispose`, the `FleetOperation` carries only the
payload, and the manager adds the transport correlation id on the
`RelayRequest`. That id is never the item's id or episode.

```rust
// FleetOperation (controller side)
DecisionKeep { keep: FleetDecisionKeep }
DecisionCloseUnanswerable { close: FleetCloseUnanswerable }

// RelayRequest (wire to the relay)
DecisionKeep { request_id: String, keep: FleetDecisionKeep }
DecisionCloseUnanswerable { request_id: String, close: FleetCloseUnanswerable }

struct FleetDecisionKeep { id: String, episode: String, actor: String, surface: String }
struct FleetCloseUnanswerable {
    items: Vec<CloseItem>,          // { request_id, episode }
    actor: String,
    surface: String,                // one of SURFACES: "web", "ios", "mac"
    controller: String,             // the controller's machine name
}
```

- The relay runs exactly what its own IPC runs:
  `CollaborationStore::keep_unanswerable` and `close_unanswerable`, with the
  registry hold, per-pair revalidation and the closure record. Nothing is
  decided on the controller.
- **Closure and audit stay in the holding store's schema.** The closure
  keeps `by: "person"`, `surface` is the surface the person actually used
  (one of the existing `SURFACES`, so `close_unanswerable` takes it
  unchanged and every reader already knows it), and `actor` is the identity
  the controller authenticated (`web:<email>`, `device:<id>`). The Fleet hop
  is recorded beside it, not in place of it: the relay's audit line gains
  `"via": "fleet", "controller": "<hostname>"` (the controller's machine
  name: its own Fleet alias is always `local`), and the controller audits the
  send as it does `ReviewDispose`. No new surface value and no closure
  schema change.
- Results are the existing `KeepOutcome` and `CloseOutcome`, carried back in
  `FleetCommandResult` as optional boxed fields like `decision_queue`.
- **The holding host's setting wins.** A relay with `[review]
  close_unanswerable = false` refuses Close all whatever the controller's
  setting is. Keep is always allowed.

### Capability and gating

- New relay capability `fleet_decision_unanswerable_v1`. The relay
  advertises it in its hello; the close half also needs the host's
  `close_unanswerable` to be on, reported as a second capability
  `fleet_close_unanswerable_v1` (the same split as the local
  `subject_check_v1` and `close_unanswerable_v1`).
- `fleet_manager.rs` refuses both operations to a relay without the
  capability, with "upgrade tend on that host", and to a read-only host.
- An older relay never receives the new variants, so it can never answer
  with something taken another way.

### Dashboard

- A remote section gains `subject_check` and `close_unanswerable` from those
  capabilities and from the host being in control mode.
- `POST /api/decisions/keep` and `/api/decisions/close-unanswerable` take an
  optional `host` alias. Without it they act locally, as today. With a
  remote alias they send the relay operation. Authorization is unchanged
  (`decision_auth_middleware`); a `decide` device can act on any control
  host the dashboard reaches, as it can for review items.
- Close all stays one list per host: the confirmation names the host and the
  count, and each host's result is reported on its own. A batch across hosts
  could report partial results explicitly, but it could never be
  all-or-none without a cross-host transaction, and one host per
  confirmation is easier to read.

### The Mac and the phone

- **Web** acts on remote hosts directly through the routes above, under the
  dashboard's own sign-in.
- **iOS** already talks to the dashboard with its device token, so it uses
  the same routes with `host`.
- **The Mac does not act on another host itself.** It has no authenticated
  native path to the dashboard today: other hosts' review items are only
  listed (`FleetDecisions.decidesReviews`), `FleetDecisionFeed.decide`
  refuses them, and the row's action opens the dashboard in the browser
  under its sign-in. A remote folded group on the Mac does the same: it is
  listed, and Keep or Close all opens that host's group on the dashboard.
  A native transport for the Mac would be its own design.
- **The CLI** (WAVE12): `tend review keep <id> --host <alias>` and `tend
  review close <id> --host <alias>` ask the local tendd, which sends the
  same `DecisionKeep` / `DecisionCloseUnanswerable` over Fleet through the
  code the dashboard uses (`crate::fleet_decisions`): the same refusals,
  the same check that the answer covers exactly what was asked, and
  `via_fleet {controller, actor: "cli:<user>", surface: "cli"}` on the
  holding host; this host records the send (`target: "fleet:<alias>"`).
  Close is one question by id; Close all stays on the dashboard.
  - **Why the CLI may have it.** The dashboard authenticates the person;
    the IPC socket cannot, and the socket refuses `fleet_command` with
    these operations so an agent cannot decide for the person on every
    host. The CLI's caller is the same Unix user who can already `ssh` to
    that host and run `tend review cleanup --questions --apply` there, so
    the path adds no authority. It still refuses an agent: the console
    origin is required, and a caller in an agent's pane or with an agent
    process (`claude`, `codex`, …) among its ancestors is refused, as is
    one whose ancestry cannot be read. A process an agent detached on
    purpose (reparented to `launchd`/`init`) is not caught; that is the
    same reach `ssh` gives it.
  - **Windows**: the IPC transport reports no caller pid there, so the
    commands are refused on a Windows controller ("could not tell who is
    asking"); use the dashboard.
  - Verified 2026-11-22 against jiun-mini with a scratch controller: from
    an agent's shell both are refused; from a terminal Keep succeeded, a
    Close of the kept question was refused before sending ("not listed"),
    and Close of another folded question closed exactly it (`by: person,
    surface: cli, actor: cli:<user>`, `via_fleet.controller` the machine
    name).

### Surfaces

- Web and iOS: the folded group of a remote host gets the same Keep and
  Close all as the local one, plus the host name in the confirmation
  ("Close 3 questions on jiun-mini? This cannot be undone."). The Mac hands
  off to the dashboard as above.
- Three cases are told apart:
  - **Both capabilities, control mode**: Keep and Close all.
  - **`fleet_decision_unanswerable_v1` but not
    `fleet_close_unanswerable_v1`** (Close all is off on that host): Keep
    stays, Close all is hidden, with "Close all is turned off on
    jiun-mini". The route refuses Close all with the same reason, not an
    upgrade message.
  - **An older relay or a read-only host**: neither, with "upgrade tend on
    that host" or "read-only host", as review items do today.

## Tests

- Relay: Keep and Close all applied on the holding host, revalidated there;
  a stale episode skipped; Close all refused when that host's
  `close_unanswerable` is off.
- Manager: refused to an old relay and to a read-only host; the
  capability split.
- Dashboard: `host` routes to the relay; a viewer refused; a `decide` device
  allowed; results per host.
- Compatibility: an old controller against a new relay (never sends the
  operations), a new controller against an old relay (buttons hidden,
  routes refuse).
- Surfaces: the three capability cases above, including base capability
  present and close capability absent (Keep offered, Close all hidden); the
  host-named confirmation; the Mac's dashboard hand-off.
- Closure: a remote close records the person's real surface and actor, and
  the relay audit carries `via` and `controller`.

## Size

Three PRs: relay and manager; dashboard routes; web, iOS and the Mac
hand-off. Not
small, which is why this is a proposal only for now.
