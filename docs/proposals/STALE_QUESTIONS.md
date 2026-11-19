# Questions nobody can be answered

Status: proposal (wave 8, 2026-11-19), revised after Codex's design review. Not built yet.

## The complaint

Tend for Mac shows questions a week or more old. Some say *Asker not listed*.
The owner asked why these questions, which look wrong, are not cleaned up.

## What the queues hold today

These were read on 2026-11-19, with every host on tend 0.10.24. The method was
read-only: `tend review queue --json`, the mailbox store opened read-only, and
`tend status --json`, through `scripts/stale-questions-audit.py` (checked in
with this proposal; it changes nothing). Ages are in days. "Live" means the
session has a row that is not Stopped.

| Host | tend's own items | Agents' questions | Asker has no live row |
| --- | --- | --- | --- |
| local | 3: 2 reply wakes (6.2 and 6.9), 1 recovery (4.9) | 0 | none |
| jiun-mbp | 11 | 1 (7.2) | 0 of 1 |
| jiun-mini | 2 reply wakes (6.9) | 12 (3.6 to 17.1) | 5 of 12 (4 sessions) |
| rtzr | 1 recovery (6.9) | 4 (6.8 to 7.7) | 3 of 4 (2 sessions) |

jiun-mbp's 11 items are:
- 6 reply wakes, 6.0 to 6.9 days old;
- 3 recoveries 10.6 to 19.2 days old, whose original request was sent from the operator console;
- 2 recoveries created 3.1 days ago about interruptions 22 and 26 days old.

On every host, `tend review cleanup` in a dry run would close **0** items. There
are three reasons.

1. **The clock counts from the item, not from the condition.** The stale rule
   (`system_asks::settle`) needs both of these:
   - the *item* is at least 7 days old;
   - every original request it covers has been in its condition for at least
     7 days.

   Items are recreated, though, and every recreation restarts the first
   clock:
   - Every reply-wake item on every host is 6.0 to 6.9 days old, while the
     replies they are about are 6.7 to 73 days old. Why they share a
     creation date of 2026-11-12 was not established; the measurement only
     shows the dates.
   - The two jiun-mbp recoveries were created 3 days ago, about episodes
     22 and 26 days old.
   - Collapsing duplicates keeps the *newest* item (`collapse`), which is
     another way an item's age restarts.

   A single recreation only delays a close until the item's seventh day, but
   items that are recreated keep pushing it back. One legacy exception: a
   parent with no condition timestamp counts as old enough
   (`waiting_since(..).is_none_or(..)`).
2. **A request the person sent is never closed for age.** The three oldest
   jiun-mbp items, 10 to 19 days old, are recoveries for requests from the
   operator console. Their recipient's binding has been unavailable for that
   long. The rule keeps them because "the decision is the person's". That
   rule is right, but an item that sits open forever is the wrong way to
   honour it.
3. **Agents' questions are out of scope by design.** The Mac app only labels
   an unlisted asker (no row for 10 minutes) and tells the person to dismiss
   each one by hand. `tend msg expire` protects human decision requests.
   Nothing ever decides that an asker is *gone*.

Nothing runs the cleanup either. `tend review cleanup` is manual, and the
daemon's own pass only covers items it opened under the current rules
(`Scope::Managed`).

## Whose absence counts

Each kind of item has one **subject**. Only the subject's absence can fold
the item.

| Item | Subject | Asker shown |
| --- | --- | --- |
| An agent's question | Its sender (`from`) | the agent |
| A recovery item | The original request's recipient (`parent.to`) | Tend recovery |
| A reply-wake item | The original request's sender (`parent.from`) | Tend reply wake |

- An item that covers several original requests folds only when the subject
  of every covered request is gone.
- A subject on another Fleet host is never judged here. The host that holds
  the question judges only sessions it observes itself.

## What "gone" means

"Not listed" is not "gone":
- A daemon restart loses rows until the next hook.
- A capped agent comes back after its reset.
- A resumed session can come back in another pane.

An item folds only when its subject is **established gone**. The evidence
comes from facts tendd records as they happen, or from a fresh scan it can
trust. A Stopped row is never read back later and reinterpreted as one of
these.

### Identity

A subject is its provider session id. Every evidence fact is keyed by:
- the host;
- the backend (tmux or rmux);
- the normalized endpoint (socket);
- the pane;
- the session id;
- when the backend reports one, the server's incarnation (its pid and start
  time).

**A live session anywhere on this host wins.** Before any fact counts, tendd
runs `recovery::exact_agent` and the resume rebinding (Codex and Claude
resumes into a new pane, `tend restore`). If the session id is bound and not
Stopped anywhere, the subject is **alive**, whatever its old pane shows.

### Evidence

**E1, ended.** The provider's own `SessionEnded` hook for that session.
- tendd writes an *end fact* when it ingests the hook, before the row can be
  reaped: identity, observed time, source.
- Only the hook counts. The reconcile sweeps (orphan, paneless, cross-host,
  pid-exit) do not write end facts.

**E2, replaced.** A real provider `Started` hook for a *different* session in
the subject's pane, on the same server incarnation.
- tendd writes a *replacement fact* in `reconcile_pane_for_started`, before
  the predecessor's row is flipped or evicted.
- An observed or synthetic `Started` does not count.

**E3, pane gone.** A complete scan of a tmux or rmux socket that answered,
with the subject's pane absent.
- The scan must be complete. herdr, cmux and zellij are partial
  observations and can never show absence.
- If the server's incarnation changed since the subject was last bound, the
  absence says nothing about the subject, so the evidence is unknown.

**E4, agent exited.** The exact conjunction `stop_rows_without_agent_process`
already requires, for the subject's pane:
- the host is complete;
- the agent kind is one the scan recognizes;
- the inactivity floor has passed;
- no primary agent process is in the pane's workload;
- the multiplexer reports a recognized shell in the foreground.

It also needs the same server incarnation. A foreground that is anything
other than a shell (a wrapper, a tool, a suspended TUI) is **unknown**, not
gone.

Never evidence:
- a missing row with no fact;
- `binding_unavailable` (its own text says process exit is not established),
  however old;
- a rate limit or usage cap;
- a failed or partial scan;
- a subject on another host.

An existing question whose subject has no fact can still become foldable
through E3 or E4 from a fresh scan, from the moment the scan shows it. No
fact is ever reconstructed backwards.

### Grace: an evidence episode

Each open item carries one durable episode record, kept on the request:

```json
"subject_check": {
  "episode": "ep_…",
  "state": "gone",
  "gone_since": "…",
  "confirmed_at": "…",
  "causes": ["pane_gone"],
  "kept_episode": null
}
```

The state machine runs on every reconcile, and every pass yields **alive**,
**gone** or **unknown**:
- **Alive** ends the episode. The record is cleared, and the next *gone*
  starts a new episode.
- **Gone** extends the episode: `confirmed_at` moves to now. On the first
  *gone*, `gone_since` and a new episode id are set.
- **Unknown** pauses the episode. If the time since `confirmed_at` exceeds 10
  minutes (a failed scan, the daemon being down, an ambiguous foreground),
  the episode resets: `gone_since` becomes the next confirmed *gone*. A
  daemon that was down 24 hours therefore cannot fold anything at start.

An item is **unanswerable** when all of these hold:
- the episode is *gone*;
- `confirmed_at` is from the current pass;
- `gone_since` is at least 24 hours old;
- the item is at least 24 hours old;
- `kept_episode` is not this episode.

A change of cause within one episode (for example pane gone, then shell,
then replaced) is the same episode. Keep therefore holds until the subject is
seen alive and then gone again.

## What happens to each kind

### tend's own items: closed automatically on a schedule, as before

These are the existing `abandoned` and `still_waiting` rules, with two
changes.

- **The clock is the condition's.** An item is stale when:
  - every covered request has been in its condition for at least 7 days;
  - nobody is left to act on it;
  - the item has been visible for at least 24 hours.

  The last condition replaces "the item is 7 days old". The legacy parent
  without a condition timestamp keeps today's behaviour.
- **The backlog is in scope.** tendd runs the full pass (`Scope::All`) at
  start and once an hour, inside the same transaction lock as the managed
  reconcile. The closer is recorded as `daemon_schedule`.

A recovery or reply wake for a request the person sent (operator console or
Fleet initiator) is never closed **for age**. It still resolves when its
condition ends, as now. When its subject (the recipient) is established gone
under the episode rules, it folds like an agent's question. Closing it, by a
person, closes only the recovery item, with a recorded dismissal. The
original task's state, its authority and its interruption record are not
changed.

On jiun-mbp, the three measured console recoveries have only
`binding_unavailable`. Unless a scan establishes E3 or E4 for their recipients,
they stay in Your turn, and the proposal says so rather than inventing
evidence.

### Agents' questions with a gone asker: folded, closed only by a person

- An unanswerable item stays open. It is listed in its own group, **Can't be
  answered (N)**, at the end of Your turn. This happens on every surface
  (Mac, web Home, iOS).
  - Its count is separate from Your turn's count, badge and menu bar count.
  - The queue lists live items before folded ones, so under truncation the
    folded items never crowd out a live question. The count of folded items
    is exact even when their list is truncated.
- The group says why for each item ("ended", "pane gone", "replaced by
  another agent", "agent exited") and since when.
- **Close all** sends the exact list it shows, as `(request id, episode)`
  pairs.
  - Inside one write transaction, tendd rechecks each pair: still open, still
    unanswerable, same episode, not kept.
  - A pair that passes is declined with the body Dismiss uses, plus a
    structured `closure`: the reason `subject_gone`, the cause, the episode,
    and who (`person`, with the surface or `cli`).
  - A pair that fails is skipped and reported back.
  - An item that arrived after the list was drawn is never included.
- **Keep** on one item sets `kept_episode` to the current episode, which puts
  it back in Your turn until a new episode.
- Nothing closes an agent's question without a person, however old.

**Reopen** is Keep. The brief's "per-item reopen" is the way back *before* a
close. A close is a decline reply, which the asker may already have read,
exactly as Dismiss is today. That is final in the protocol and is not
reversed. Undoing a delivered decline would need a second, independent answer
revision, which the mailbox does not have, and inventing one is out of scope.
History shows every close with its reason, so nothing disappears unseen.

### Alternatives considered

- **Auto-close agents' questions after N days with the asker gone.** This is
  the least work for the person, but a question can still matter with its
  asker gone: an approval for a deploy, or a choice another agent will pick
  up from the mailbox. It would turn a classification error into a lost
  decision. It is rejected; a later config switch could offer it.
- **Label only (today).** Rejected: this is the complaint.
- **Hide them, with only a count.** This keeps Your turn clean, but the
  person cannot tell "nothing waits" from "things wait that I cannot see".
  The folded group is the same idea, visible.
- **Re-route the question to another agent in the same Work.** An agent's
  question is about its own state, so another agent's answer would not apply.
  Out of scope.

## Wire format and compatibility

- **Queue.** Items gain an optional `unanswerable: {cause, episode, since}`
  object, and the queue gains `unanswerable_count`.
  - Old clients ignore unknown fields and keep listing the item as today, in
    Your turn.
  - Rust, the Swift `TendLenient` decoders and `needs-model.mjs` all ignore
    unknown keys. Tests decode a new server's queue with each of them.
- **History.** No new variant is added to the history response old readers
  decode.
  - A person's close is a declined human answer. It already decodes as one,
    and the new `closure` object is an optional field.
  - tend's own automatic closes are listed only when the caller asks with
    `include_closures`. New clients send it, and a Fleet coordinator sends it
    only to a host that advertises `decision_history_closures_v1`.
  - An old client therefore sees exactly the history it sees today, without
    tend's closures. An old coordinator never receives a variant it cannot
    decode.
  - Tests cover an old client against a new server, and an old Fleet
    coordinator against a new host.
- **Close all and Keep** are new IPC operations behind
  `subject_check_v1`, mirrored as dashboard routes for the web and the
  phone (operator for the web; `operate` scope for a device, as with
  decisions). Old clients do not show the buttons.

## The CLI

`tend review cleanup` covers both kinds, and the default is a dry run:

```text
would close 6 of tend's own items (stale: nobody left to act on it …)
3 questions can't be answered (subject gone 24h+); not closed:
  req_…  8.7d  information  pane %42 gone since 2026-11-16 (otpeek)
  …
run with --apply to close tend's items; add --questions to also close these 3
```

- `--apply --questions` does what Close all does, for the list just shown.
- `tend review keep <id>` keeps one item.
- `--json` gives both lists.
- The command never touches an item whose subject is unknown or only
  unlisted.

## Build plan

1. **Daemon and CLI** (one PR):
   - end and replacement facts recorded at ingestion;
   - the E1 to E4 evaluation, with the episode record on the request;
   - the condition-age stale rule, and the hourly full pass;
   - the `unanswerable` queue field, and `include_closures` in history;
   - Close all and Keep (IPC and dashboard);
   - `tend review cleanup --questions` and `tend review keep`.
2. **Surfaces:** the folded group, Close all and Keep on Mac Home, web Home
   and iOS Home.

## Tests

Fixtures are shaped like the measured items:
- a reply wake recreated long after its reply (condition 73 days, item 6.9);
- a recovery recreated 3 days ago about a 26-day episode;
- a console-sent recovery with only `binding_unavailable` for 19 days, which
  stays;
- the same one with its recipient's pane gone, which folds and is never
  auto-closed.

Each identity and evidence case gets its own fixture:
- an agent question whose pane is gone (E3);
- a pane that now runs a shell (E4);
- a pane that now holds another hooked session (E2);
- a session that ended (E1);
- a resume into a new pane (alive);
- `tend restore` (alive);
- a tmux server restart that reuses the pane id (unknown);
- the same pane id on two sockets;
- a partial (herdr) scan;
- a failed socket;
- a daemon down 24 hours (the episode resets);
- a rate-limited agent;
- an asker on another host.

Races:
- Close all against a reply, against Keep and against a resume;
- an item arriving after the list was drawn;
- a change of cause within one episode, with Keep holding.

Compatibility:
- old Rust, Swift and JS decoders against the new queue and history;
- an old Fleet coordinator against a new host.

All development uses a scratch daemon and fixtures. No item on the owner's
hosts is closed, answered or decided.
