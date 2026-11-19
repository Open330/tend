# Questions nobody can be answered

Status: proposal (wave 8, 2026-11-19). Not built yet.

## The complaint

Tend for Mac shows questions a week or more old. Some say *Asker not listed*.
The owner asked why these questions, which look wrong, are not cleaned up.

## What the queues hold today

These were read on 2026-11-19, with every host on tend 0.10.24. The method was
read-only: `tend review queue --json`, the mailbox store opened read-only, and
`tend status --json`. The script that read them was not checked in. Ages are
in days. "Live" means the session has a row that is not Stopped.

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

   Items are recreated, though:
   - Every reply-wake item on every host was created on 2026-11-12, when the
     backlog was moved to managed items. Their replies are 6.7 to 73 days old.
   - The two jiun-mbp recoveries were recreated 3 days ago, about episodes
     22 and 26 days old.

   So each recreation restarts the clock, and the 7 days never pass for the
   item even when the condition is weeks old.
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

## What "gone" means

"Unlisted" is not "gone":
- A daemon restart loses rows until the next hook.
- A capped agent comes back after its reset.
- A pane on another host is not ours to judge.

The rules below act only on an asker *established* gone. The evidence is
checked on the host that holds the question, about a pane on that host, from a
scan that succeeded.

Each of these counts as **established gone**:
- **G1 ended:** the session's own hook said it ended (`SessionEnded`, asserted).
- **G2 replaced:** the pane now holds another agent session, from a real
  `Started` in that pane.
- **G3 pane gone:** the asker's pane is absent from a successful scan of its
  tmux or rmux socket.
  - tmux reuses pane ids only after the server restarts.
  - A reused id holding a different session is G2. A reused id holding the
    same session is the asker alive again.
- **G4 agent exited:** the pane exists, but its foreground command is a shell
  or another program, not the agent, and no agent row is bound to it.

These are **not** established, so the asker counts as unlisted at most:
- No row and no record (for example, a daemon restart).
- The socket cannot be scanned.
- The pane still runs the agent's process with no row bound.
- `binding_unavailable`, whose own text says "process exit is not
  established".
- A rate limit or usage cap.
- An asker on another host.
- The orphan, cross-host and paneless sweeps, which flip rows to Stopped by
  inference.

Gone must also hold **continuously** for a grace period:
- tendd records the first time it saw the evidence on the request,
  `asker.gone_since` with its cause.
- It clears that record the moment the asker is live again.
- A question counts as *unanswerable* only once `gone_since` is 24 hours old
  and the question itself is 24 hours old.

24 hours covers a restart, a re-attach and a night away. A capped agent is not
gone under any of G1 to G4.

The evaluation runs in the daemon's reconcile, and every host evaluates only
its own store. That is also how Fleet behaves: each host cleans its own
questions, and a Fleet client only reads what each host says.

## What happens to each kind

### tend's own items: closed automatically, on a schedule

- **The clock is the condition's.** An item is stale when:
  - every original request it covers has been in its condition for at least
    7 days (unchanged);
  - nobody is left to act on it (`abandoned`, unchanged);
  - the item has been visible for at least 24 hours.

  The last condition replaces "the item is 7 days old". It means a
  recreated item cannot disappear the moment it appears, and the clock no
  longer restarts on every recreation.
- **The backlog is in scope.** The daemon runs the full pass (`Scope::All`,
  today only `tend review cleanup`) once an hour and at start. The closer is
  recorded as `daemon_schedule`, and the reason is already on the row
  (`SystemAskClosure`).
- **Visible in history.** `decision_history` lists tend's closures as their
  own kind, `closed_by_tend`, with the reason and what the item was about.
  That way a closed item can be found from the app's History as well as from
  the mailbox.
- **A request the person sent** (operator console or Fleet initiator) is still
  never closed. When its recipient is established gone (G1 to G4) with the
  grace above, it is treated like an agent's question with a gone asker,
  below. The decision stays the person's, and it leaves Your turn.

### Agents' questions with a gone asker: folded, never deleted silently

The proposed default:
- An unanswerable question stays open. It moves out of Your turn into a
  folded group, **Can't be answered (N)**, at the end of Your turn. This
  happens on every surface (Mac, web Home, iOS), because each one reads it
  from the queue.
- The group says why for each item ("ended", "pane gone", "replaced by
  another agent", "agent exited", and when).
- **Close all** closes every question in the group. Each one is declined with
  the body Dismiss uses and a structured `closure` that records:
  - the reason `asker_gone`;
  - the cause;
  - who did it (`person`, with the surface or `cli`).
- **Keep** on an item puts it back in Your turn. The record is
  `asker.kept_at`, and the fold does not take it again until the asker's
  evidence changes, for example it comes back and goes again.
- A closed item is listed in History with its reason, and `tend review reopen
  <id>` reopens it to Queued, recording that it was reopened. A reopened item
  is never folded or closed again for the same evidence.
- Nothing closes an agent's question without a person, however old it is.

The queue item carries the classification. Clients that predate it ignore the
unknown field and keep listing the item as they do now:

```json
"unanswerable": { "cause": "pane_gone", "since": "…", "pane": "%42" }
```

### Alternatives considered

- **Auto-close agents' questions after N days with the asker gone.** This is
  the least work for the person, but a question can still matter with its
  asker gone: an approval for a deploy, or a choice another agent will pick
  up from the mailbox. It would turn a classification error into a lost
  decision. It is rejected as the default; a later config switch could offer
  it.
- **Label only (today).** Rejected: this is the complaint.
- **Hide them entirely, with a count in the header.** This keeps Your turn
  clean, but it moves the decision out of sight, and the person cannot tell
  "nothing waits" from "things wait that I cannot see". The folded group is
  the same idea, visible.
- **Re-route the question to another agent in the same Work.** This is
  attractive, but an agent's question is about its own state, so the answer
  would not apply. It is out of scope.

## The CLI

`tend review cleanup` covers both kinds, and the default is a dry run:

```text
would close 6 of tend's own items (stale: nobody left to act on it …)
3 agents' questions can't be answered (asker gone 24h+); not closed:
  req_…  8.7d  information  pane %42 gone since 2026-11-16 (otpeek)
  …
run with --apply to close tend's items; add --questions to also close these 3
```

- `--apply` closes tend's own items.
- `--apply --questions` also closes the unanswerable questions, exactly as
  Close all does.
- `--json` gives both lists.
- The command never touches a question whose asker is only unlisted.

## Build plan

1. **The daemon and the CLI** (one PR):
   - the asker evidence, with `gone_since` and `kept_at` persisted on the
     request;
   - the condition-age stale rule, and the hourly full pass;
   - `closed_by_tend` in history, and `unanswerable` on queue items;
   - routes for close-all, keep and reopen: IPC, plus the dashboard's HTTP
     for the web and iOS;
   - `tend review cleanup --questions` and `tend review reopen`;
   - fixture tests shaped like the measured items, listed under Tests.
2. **The surfaces** (one PR each, or one together): the folded group, Close
   all and Keep on Mac Home, web Home and iOS Home. History shows the
   closures there too.

## Tests

The fixtures are shaped like the measured items:
- a reply wake recreated 7 days after its reply arrived, so the condition is
  73 days old and the item 6.9;
- a recovery recreated 3 days ago about a 26-day episode;
- a console-sent recovery with its recipient's binding unavailable for 19 days;
- an agent's question whose pane is gone, one whose pane now runs a shell, one
  whose pane holds another agent, one whose asker is only unlisted after a
  daemon restart, one that is rate-limited, and one from another host.

Each fixture asserts which items close, which fold and which stay. Kept and
reopened items must not fold or close again for the same evidence.

All development uses a scratch daemon and fixtures. No item on the owner's
hosts is closed, answered or decided.
