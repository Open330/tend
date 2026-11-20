# Questions nobody can be answered

Status: agreed design (wave 8, 2026-11-19, Codex agreed on 43531a1a). Built: tend's own items (#638); evidence, episodes, folding, Keep and Close all in the daemon, IPC, dashboard and CLI (#642); Close all on by default by the owner's decision. The surfaces (step 2) are in #643.

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
  absence says nothing about the subject, so the evidence is unknown, unless
  present state establishes it (see "Present state" below).

**E4, agent exited.** The exact conjunction `stop_rows_without_agent_process`
already requires, for the subject's pane:
- the host is complete;
- the agent kind is one the scan recognizes;
- the inactivity floor has passed;
- no primary agent process is in the pane's workload;
- the multiplexer reports a recognized shell in the foreground.

It also needs the same server incarnation, or present state. A foreground
that is anything other than a shell (a wrapper, a tool, a suspended TUI) is
**unknown**, not gone.

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
  only the grace clock restarts: `gone_since` becomes the next confirmed
  *gone*. The episode id and `kept_episode` are unchanged, because nothing
  showed the subject alive. A daemon that was down therefore cannot
  fold anything at start, and an item kept before the outage stays kept.

An item is **unanswerable** when all of these hold:
- the episode is *gone*;
- `confirmed_at` is from the current pass;
- `gone_since` is at least 15 minutes old (the grace, below);
- the item is at least 15 minutes old;
- `kept_episode` is not this episode.

A change of cause within one episode (for example pane gone, then shell,
then replaced) is the same episode. Only positive *alive* evidence ends an
episode. Keep therefore holds until the subject is seen alive and then gone
again; an outage alone never starts a new episode.

### Bindings survive a daemon restart

E3 and E4 on tmux need the server incarnation the subject was bound on: a
pane id is unique only within one incarnation. tendd learns a binding by
seeing the session's live row in its pane in a complete scan, and kept it
only in memory. After a daemon restart, a subject that never reappears had
no binding, so its absent pane said nothing, and its question never folded
unless E1, E2 or a stopped tmux server showed it gone.

The binding is therefore also kept on the open item, next to its episode:

```json
"subject_bindings": {
  "<session id>": { "pane": "%42", "endpoint": "/tmp/tmux-501/default",
                    "incarnation": { "pid": 812, "start_time": 1763500000 } }
}
```

- **Written** by the subject check pass, for each subject of each open human
  item, whenever tendd has an in-memory binding for it and the stored one
  differs. A write happens only when a subject is first seen or moves.
- **Same pane and endpoint keeps the stored incarnation**, as the in-memory
  map already does: the first incarnation the session was seen on wins. A
  row reloaded after a restart into a pane id a new server reuses therefore
  never re-stamps the old binding with the new incarnation.
- **Another pane or endpoint replaces it**: the session was seen live
  elsewhere.
- **Read** when tendd has no in-memory binding for the subject. It counts
  only for the pane being judged (the row's or the participant's), exactly
  as the in-memory one does.

Nothing else changes. A stored binding is a record of where tendd saw the
session alive, not evidence that it is gone. The pane must still be absent
from a complete scan of a server with that same incarnation (E3), or hold
a shell with no agent process (E4), and the grace still applies. A
tmux server restarted since keeps this evidence unknown, as before. An item
asked before this change has no stored binding until its subject is seen
again; present state (next) covers it.

Old daemons ignore the field. It is skipped when empty.

### Present state: an asker gone before its binding was stored

Measured on 0.10.28 (2026-11-20): of jiun-mini's 18 open agent questions, 10
were 7 to 9 days old and their asker had no live row. `tend review cleanup
--questions` listed none of them. Four had an `ended` fact and were inside the
24-hour grace; the other six had no stored binding (asked before it was
kept), and their tmux server had been restarted since, so E3 and E4 said
nothing. rtzr (4) and jiun-mbp (1) had the same pattern.

A binding is history: where tendd saw the session. What the person asks is
about now: **the asker is not running anywhere on this host, and its pane no
longer holds it.** tendd can establish that from the present alone, but only
from a walk that saw everything:

- **The walk is complete** (`PaneScan::processes_walked`): every backend's
  scan was complete (a herdr, cmux or zellij observation, or one failed
  socket, makes it partial), the process table was read (a failed `ps`
  reads as empty, not as "nothing runs"), every pane had a root process,
  and no pane's walk hit its node limit. The reconcile pass already walks
  every pane's process tree once; it now reports whether that walk was
  whole.
- **The asker is not running**: no bound, not-Stopped row for its session id
  (as before), and no process in any pane was started on it: `codex resume
  <id>`, `claude --resume <id>` / `-r` / `--session-id`, kept per pane as
  `ScannedPane::resumed`. A session found there is **alive**, whatever its
  row says, and that counts from any walk, complete or not. That is the swap
  and `tend reload` case: the same session id in a new process, in its old
  pane or a new one, possibly under another account. Its row comes back on
  the first prompt; until then its process is the evidence. On jiun-mini, %8
  was exactly this: an idle `codex --no-daemon resume 01a132f3…` whose row
  was synthetic. A pane check alone would have called it replaced.
- **Every agent on the host is someone else.** A command line names the
  session only when it was started by id. `claude --continue`, a session
  picked from a list, or a thread tendd has not bound runs under no name it
  can read, and could be the asker. So every pane whose walk found an agent
  process must be accounted for: a named, not-Stopped row in it, or an
  explicit resume id. While one is not, anywhere on the host, only the bound
  incarnation counts (Codex review of #686).

Then the pane needs no history, on any server incarnation:

- it is absent (E3, `pane_gone`);
- or a different session tendd can name holds it with a live row (E2,
  `replaced`);
- or it runs a shell and no agent, and no live row holds it (E4,
  `agent_exited`).

Otherwise the old rule stands: only the bound incarnation counts. Old
daemons ignore both new fields.

Why this is safe now and was not before: the incarnation rule existed so that
a session restored into a new pane tendd had not tied to it yet was not
called gone from its old pane id. Restores and swaps relaunch the session by
id, which the walk shows directly; a session relaunched any other way is an
unaccounted agent, which stops the inference. Close all reads the same
evidence again inside its transaction, so it closes nothing these rules
would not fold.

Measured read-only on 2026-11-20 with these rules: no host had an
unaccounted agent process.

### Grace: long enough for a swap, not a day

The 24-hour grace kept week-old questions in Your turn for a day after their
asker exited, and the owner wants them out when the agent is gone. The grace
only has to absorb the ways a live session is briefly unseen:

| What | How long it is unseen | Covered by |
| --- | --- | --- |
| Account swap, `tend reload` | a minute or two; the resume process is seen at once | the resume set, then 15 min |
| tendd restart | one or two missed passes | the durable episode; `UNKNOWN_GAP` (10 min) |
| Reboot then `tend restore` | minutes, while the person restores | 15 min; later, it unfolds the moment it is back |
| `/clear`, a new session in the pane | it is a different session | not absorbed: the asker really is gone |

`SUBJECT_GONE_GRACE` and `ITEM_MIN_AGE` are therefore **15 minutes**: three
confirmation windows (`CONFIRMATION_FRESH`, 5 min) and longer than the
unknown gap, so one missed scan cannot fold anything, and a swap finishes
well inside it. Longer would only keep questions nobody can answer in the
person's turn. A fold is reversible: the subject seen alive ends the
episode and the item returns to Your turn. It stays reachable under
**Can't be answered** with Keep, and nothing closes it without a person.

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
  reconcile. The closer is recorded as `daemon`, a value older readers
  already decode, rather than a new one.

A recovery item for a request the person sent (operator console or Fleet
initiator) is closed **for age** only when its recipient has been
**vanished** (`binding_unavailable`: no agent row at all, live or Stopped)
for the whole 7 days. That clock is the reason's, not the episode's: an
episode that began as an error a month ago and lost its recipient's row
today counts from today. The interruption records when its current reason
began (`reason_since`, reset whenever the reason changes); an episode from
before the field is stamped at the first reconcile after the upgrade, so its
week starts then. On jiun-mbp three such items were 11 to 20 days old: four console
dispatches to a Codex session with no row since 2026-11-08, three console
requests from September to two Claude sessions with none since 2026-10-31.
Their answer "records a decision; it does not restart, cancel, or spend";
with no recipient there is nothing for it to decide. The original request
is not changed: it stays the person's, still listed as interrupted, to
resend or retire. A recipient that is only failing, rate-limited or stopped
still has a row, so its item stays for the person however old. It still
resolves when its condition ends, as now. When its subject is established gone under the episode rules,
it folds like an agent's question. For a recovery item the subject is the
original request's recipient; `binding_unavailable` alone never counts.

Closing it is a person's decline of the recovery item. As with any answer to
a recovery item, the decline is copied into the original request's
interruption decision (`recovery.rs`, `info.decision.clone_from`), so the
original request records that the person dismissed it. Its status, its
execution authority and the task itself are not changed, and nothing is
restarted, cancelled or retried. A regression test pins this.

Reply wakes keep today's `abandoned` rule, which exempts only a console
sender. Adding the Fleet-initiator exemption to reply wakes is not part of
this change.

The three jiun-mbp console recoveries fold at once (their recipients' panes
are gone or held by other named sessions) and close under the
vanished-recipient rule a week after the upgrade, when their `reason_since`
was first stamped.

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

**Reopening a closed item is not built. That needs the owner's decision.**
The brief asks for a per-item reopen. This design offers Keep, which is the
way back *before* a close, and that is not the same thing. A close is a
decline reply, which the asker may already have read, exactly as Dismiss is
today, and it is final in the protocol. Reopening it would need a second,
independent answer revision that keeps the delivered decline and the
history intact. The mailbox has no such revision, and designing one is a
separate protocol change.

The choice for the owner:
- (a) accept that closing is final, as Dismiss already is, with History
  showing every close and its reason;
- (b) commission the answer-revision design first, and ship folding and Keep
  without Close all until then.

**Decision (2026-11-19): (a), by the owner.** Asked by the coordinator
whether Close all may be irreversible, the owner answered in their own
words: "모두닫기는 왜 꺼집니까? 되돌릴수없음만 표시해주면 사용자가닫을수도있을것같습니다"
("Why is Close all turned off? If it just shows that it cannot be undone,
the user may as well close them."). So:

- Close all is **on by default** (`[review] close_unanswerable = true`);
  `false` turns it off.
- Its one confirmation, on Mac, web and iOS, names the count, says each
  asking agent gets a reply that the person dismissed it, and says plainly
  that this cannot be undone.
- It never includes an item whose evidence is younger than the grace
  or that was kept, and `tend review cleanup --questions` stays a dry run
  unless `--apply` is given.
- Reopening a closed item is not offered: a close is final, as Dismiss is.

Folding and Keep do not depend on this choice.

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
  phone. They are authorized like the existing decision routes
  (`decision_auth_middleware`): an operator on the web, or a paired device
  of either scope. A `decide` device can already decline a question, and
  Close all is that same decline for an explicit list, while Keep only
  changes where an item is listed. Tests cover a `decide` device succeeding,
  a viewer being refused, and a device token being refused on an unrelated
  Work-control route. Old clients do not show the buttons.

## The CLI

`tend review cleanup` covers both kinds, and the default is a dry run:

```text
would close 6 of tend's own items (stale: nobody left to act on it …)
3 questions can't be answered (subject gone); not closed:
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

Close all is on by default since the owner's decision above.

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
- a daemon down for hours, where the grace clock restarts but the episode
  does not;
- Keep, then unknown for hours, then gone again with no alive in between,
  which stays kept;
- a recovery close, after which the original request records the dismissal
  in its interruption decision, with its status unchanged and nothing
  restarted;
- a `decide` device closing and keeping, a viewer refused, and a device
  refused on Work control;
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
