# Keep and Close all across the Fleet

Status: proposal. Not built. Follow-up to
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

Two new `FleetOperation` variants, both `is_control()`:

```rust
DecisionKeep { request_id: String, keep: FleetDecisionKeep }
// { id, episode, by }
DecisionCloseUnanswerable { request_id: String, close: FleetCloseUnanswerable }
// { items: [{ request_id, episode }], by }
```

- The relay runs exactly what its own IPC runs:
  `CollaborationStore::keep_unanswerable` and `close_unanswerable`, with the
  registry hold, per-pair revalidation and the closure record. Nothing is
  decided on the controller.
- `by` is the operator actor the controller authenticated
  (`web:<email>`, `device:<id>`). The relay records it as the closer and
  audits `fleet:<controller alias>` as the surface.
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

### Dashboard and IPC

- A remote section gains `subject_check` and `close_unanswerable` from those
  capabilities and from the host being in control mode.
- `POST /api/decisions/keep` and `/api/decisions/close-unanswerable` take an
  optional `host` alias. Without it they act locally, as today. With a
  remote alias they send the relay operation. Authorization is unchanged
  (`decision_auth_middleware`); a `decide` device can act on any control
  host the dashboard reaches, as it can for review items.
- Close all stays one list per host: the confirmation names the host and the
  count, and each host's result is reported on its own. A Close all that
  spans hosts would need a cross-host transaction, which the Fleet does not
  have, and partial success would be hard to read.
- The Mac app sends a remote Keep or Close all through the dashboard routes,
  as it already decides other hosts' review items there
  (`FleetDecision.decidesReviews`), and keeps using IPC for its own host.

### Surfaces

- Mac, web and iOS: the folded group of a remote host gets the same Keep and
  Close all as the local one, plus the host name in the confirmation
  ("Close 3 questions on jiun-mini? This cannot be undone.").
- A host that cannot take them (older, read-only, or Close all off there)
  shows the folded group without the buttons and one line saying why, as
  review items do today.

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
- Surfaces: the remote folded group with and without the buttons, and the
  host-named confirmation.

## Size

Three PRs: relay and manager; dashboard and IPC; Mac, web and iOS. Not
small, which is why this is a proposal only for now.
