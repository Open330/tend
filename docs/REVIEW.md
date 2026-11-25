# Review items

A review item is an explicit request that one result be looked at. It stays
visible until someone records what happened to it. Design and rationale:
[proposals/REVIEW_DEBT.md](proposals/REVIEW_DEBT.md).

tend never creates an item from what it observes. A turn stopping, a process
exiting, a Run completing or a branch merging does not mean a result is ready
or reviewed. Items come only from a claim: someone, a user or an agent, asked
for review.

## Commands

```sh
# Ask for review. Exactly one target.
tend review request --work tend/storage --alias impl \
  --summary "Check the migration and the retry path" --anchor commit:1a2b3c
tend review request --session <session-id> --provider codex --summary "…"
tend review request --request <collaboration-request-id> --summary "…"

# Ask for a decision, with the context to make it from the card.
tend review request --work tend/storage --alias impl \
  --summary "Storage migration ready" \
  --decision-needed "Merge now or wait for the perf run?" \
  --option "merge now" --option "wait for perf" \
  --recommend "merge now: the perf run only covers the read path" \
  --evidence commit:1a2b3c --evidence https://github.com/o/r/pull/12 \
  --evidence "test-pass:cargo test -p tend review"

tend review list            # open items, newest first, 50 per page
tend review list --oldest   # oldest first (where the status-line age comes from)
tend review list --all      # include disposed items
tend review list --offset 50   # next page; a page that has more says so
tend review show rv_…
tend review queue           # everything waiting on you, in one list
tend review history         # what was decided: dispositions, answers, tend's closures
tend review cleanup         # what of tend's own items would close now (--apply closes it)

tend review accept   rv_… [--note "…"]
tend review return   rv_… --note "tests missing"   # sent back for more work
tend review withdraw rv_… [--note "…"]
```

Every command takes `--json`. `request` and the dispositions take
`--as user|agent` (default `user`) and an optional `--label`.

## From agents

An agent files a result record with the `tend_file_result` MCP tool (target:
its managed Work alias, else its session), or inside a pipeline with

```sh
tend work done --summary "Migration and retry path done" --commit 1a2b3c \
  --evidence "test-pass:cargo test -p tend review" \
  --decision-needed "Ship behind the flag or on by default?" \
  --option "behind the flag" --option "on by default" --recommend "behind the flag"
```

which files the item, keyed by Work, alias and pipeline generation, and
reports the alias done in the same tendd operation (`tend_file_result` does
the same with `complete: true`). A refused record leaves the alias open; a
crash between filing and completing is finished at the next start. An older
tendd without `work_done_record_v1` gets the record first and the completion
second, as before.

## Decision fields

An item may carry a `decision`: `needed` (the question), `options` (up to
8 one-line choices), `recommendation`, and `evidence` (up to 16 typed
pointers). Evidence kinds:

| Kind | Shorthand | Fields |
| --- | --- | --- |
| `commit` | `commit:SHA` | `sha` (4-64 hex), `repo` (absolute checkout path) |
| `pull_request` | `pr:URL` or a bare `https://` URL | `url` |
| `diff` | `diff:A..B` | `range` (revision syntax only), `repo` |
| `test` | `test:CMD`, `test-pass:CMD`, `test-fail:CMD` | `command`, `passed`, `tail` (last 2 KiB of output, escapes removed) |
| `path` | `path:PATH` | `path` |

`commit:` and `diff:` evidence records the current directory's git
checkout unless `--repo` names one. Every field is the asker's declaration;
tend validates its shape, never its truth. Human requests
(`tend_request_attention`) carry the same `decision` object next to their
`body`. Storing it needs a tendd advertising `review_decision_v1` (review
items) or `collaboration_human_decision_v1` (human requests).

## Semantics

- **Retries.** `--key` makes a request idempotent: the same key and target
  return the existing item. The same key for another target is refused.
- **Generations.** Each new request for the same target (session, Work +
  alias, or collaboration request) takes the next generation. Accepting
  generation 1 does not affect generation 2.
- **Dispositions are final.** `accepted`, `returned` and `withdrawn` are
  recorded once. Repeating the same one is a no-op; a different one is
  refused. To review again, request again.
- **Declared, not proven.** Actors are what the caller declared. Local IPC
  does not authenticate a person, so `accepted` means someone said they
  accepted the result, not that a human understood it.
- **Targets are descriptors.** An item outlives its pane, session or
  transcript. Nothing retires an item except an explicit disposition.
- **Evidence is printed, not opened.** `tend review show` lists the command
  that opens each piece of evidence (and a `commit:` anchor): `git show` /
  `git diff` (with `--stat`) in the recorded checkout, `gh pr view` /
  `gh pr diff` for a pull request, the command to rerun a test. tend runs
  none of them: the checkout is the asker's declaration and git reads
  repository config that can run programs (signature verifiers, lazy
  fetches), so opening evidence is the person's own act, in their shell.
- **No control characters.** Identifiers, labels, anchors and keys refuse all
  control characters; summaries and notes allow newlines and tabs only. Every
  stored string is printed by the CLI, so terminal escapes are refused at the
  boundary.
- **Bounded pages.** A list returns at most 500 items and reports whether more
  follow, so the oldest item is always reachable.

## One queue

`tend review queue` (IPC `decision_queue`, capability `decision_queue_v1`)
returns everything pending on the person as one ordered list:

1. human requests (`tend_request_attention`), with their options and
   recommendation, oldest first;
2. agents blocked in their pane (error, choice, input, then a turn that
   ended on a question), oldest first;
3. open review items, oldest first.

The order is how much is blocked, then age. `tend attend --list` shows the
same human requests and the open review count under the agents it can jump
to. `tend review history` (IPC `decision_history`) lists decided review items
with disposition, note and time, and answered human requests with the
answer, newest first. Both reads take `--limit` (at most 500, and at most
4 MiB of records) and say when a source had more, or could not be read,
instead of showing it as empty. Human requests from every room are an
operator-console read, as for any listing past one's own mailbox: the CLI
asks as the console and the read is written to the collaboration audit
ledger; an agent's origin gets reviews and waiting agents only.

### tend's own items close themselves

tendd opens some items itself: peer recovery (a request's recipient is
interrupted) and reply wakes (a reply arrived for a sender that is gone).
They are listed after agents' questions and results, and tendd closes them,
without an answer and with the reason recorded on the row, when they can no
longer matter:

- the original request is gone, or no longer holds the condition the item
  was about;
- another open item about the same session covers it (merged);
- nobody is left to act on an answer, and every original request it covers
  has been in its condition for a week. The week counts from the condition,
  not from the item, because items are recreated. An item is listed for at
  least a day before it can close this way. A recovery item for a request
  the person sent is never closed for age.
- the session it is about is established gone and the item has folded
  under "Can't be answered" (below), unless the person kept it: the asker
  is tend, and nothing can act on an answer. A recovery item for a request
  the person sent waits until its recipient has been gone a week. This
  closure runs in the hourly pass and `tend review cleanup`, rechecked
  with the registry held as Close all is, and is recorded as `stale` with
  `subject_gone: <cause>` (a field, so older readers still read the row);
  a recovery item closed this way is asked again only once its recipient
  is live and still interrupted.

tendd applies the rules on every reconcile to the items it opened under
them, and to every item, the backlog included, at start and every hour.
`tend review cleanup` shows what the same pass would close now, and
`--apply` runs it. Agents' own questions are never touched.

The history lists these closures when the reader asks for them:
`decision_history` with `include_closures` (capability
`decision_history_closures_v1`), `GET /api/decisions/history?closures=true`,
and over Fleet only to a relay with `fleet_decision_history_closures_v1`.
Each is `"kind": "closed_by_tend"`, with `reason`, `reason_text`,
`closed_by`, `into` for a merge, and `closed_at`. Without the flag the history
is exactly what older readers decode. `tend review history` shows them;
with `--json` it lists them only with `--closures`, so scripts that parse
it see the shape they already know.

### Questions nobody can answer

An open question whose **subject** is established gone is folded out of the
person's turn into its own group, **Can't be answered**, listed after every
live item. Only a person closes it. The subject is the asking agent for an
agent's question (`from`), the original request's recipient for a recovery
item, and its sender for a reply wake; an item that covers several requests
folds only when every subject is gone. Subjects on another Fleet host, the
operator console and addresses still waiting for a session are never judged.

A live session anywhere on this host (the session's own row, after a resume
into another pane or `tend restore`) means alive. Otherwise only these count:

| Cause | Evidence |
| --- | --- |
| `ended` | the provider's own `SessionEnded` hook, recorded when tendd ingests it |
| `replaced` | a real provider `Started` hook for another session in the pane, recorded before the old row changes; it counts only if the first scan after the hook finds the tmux server on the incarnation the old session was bound on; or, from present state, another session tendd can name holds the pane with a live row |
| `pane_gone` | the pane is absent from a complete scan of a socket that answered: for tmux, on the incarnation the session was bound on (kept on the question, so it survives a daemon restart), or from present state; for rmux, which reports none, from its running endpoint; or the socket answers that no server runs there |
| `agent_exited` | what tendd already requires to stop a row with no agent process (a complete host, a recognized agent, an hour idle, no agent process, a shell in the foreground), on the same server incarnation, or from present state with no live row in the pane |

**Present state** needs a complete walk of every pane's processes on every
backend, with every agent process found accounted for by a named live row
or an explicit resume id (an unnamed agent anywhere, such as `claude
--continue`, could be the asker). Then a session with no live row and no
process started on it (`codex resume <id>`, `claude --resume <id>`) is
judged by its pane alone: absent, held by another named session, or a
shell. A session that is resumed, even unprompted and in another pane or
account, is alive.

Nothing else is evidence: not a missing row, `binding_unavailable`, a rate
limit, a failed or partial scan (herdr, cmux and zellij never show absence),
or a subject on another host. The facts (`ended`, `replaced`) are kept in the
mailbox database for 30 days (a failed write keeps them queued for the next
pass); the reconcile sweeps write none. The server
incarnation is the tmux server's `#{pid}` and `#{start_time}`, read in the
same `list-panes` call as the panes; rmux does not report one, so on rmux only
`ended` and `pane_gone` apply.

Every reconcile pass keeps an evidence episode on the request
(`subject_check`: `episode`, `state`, `gone_since`, `episode_since`,
`confirmed_at`, `causes`, `kept_episode`). Alive ends it; gone extends it;
unknown pauses it, and more than 10 minutes of unknown (a daemon restart, a
host asleep) restarts only the grace clock `gone_since`, never the episode
or `episode_since`, when it began, which is the "gone since" every surface
shows. A row from before `episode_since` takes confirmed evidence only:
its own clocks or the subject's recorded end hook, never before the item.
An item is unanswerable when the episode is gone and confirmed by a pass in
the last 5 minutes, has been for 15 minutes, the item is at least 15 minutes old, and
the person has not kept this episode. A daemon that was down cannot fold
anything at start.

On the wire (`subject_check_v1`), a folded `human_request` carries
`unanswerable: {cause, episode, since, pane?}` and the queue
`unanswerable_count`, exact even when the list is cut; folded items come
after every live one, so they never crowd a live question out. Older readers
ignore both and list the item as before.

- **Keep** (`decision_keep {request_id, episode}`, `POST
  /api/decisions/keep`, `tend review keep <id>`) puts the item back in the
  person's turn until its subject is seen alive and then gone again.
- **Close one** (`tend review close <id>`) closes exactly that question, as
  Close all below would.
- **On another Fleet host**, `tend review keep <id> --host <alias>` and
  `tend review close <id> --host <alias>` (`decision_keep_on_host`,
  `decision_close_on_host`, `decision_on_host_v1`) go over the same Fleet
  route as the dashboard's `host`, with the same refusals, and the holding
  host records `via_fleet`. They are the person's alone: the console origin,
  and a caller that is neither in an agent's pane nor under an agent
  process, is required.
- **Close all** (`decision_close_unanswerable {items: [{request_id,
  episode}], by_label?}`, `POST /api/decisions/close-unanswerable`, `tend
  review cleanup --questions --apply`) declines exactly the listed pairs, in
  one transaction, each still open, still unanswerable, in that episode and
  not kept, and still established gone by the registry's evidence read
  again inside that transaction (a resume, a returned pane or a stale scan
  stops it); the rest come back in `skipped` with why. An item covering a
  request that cannot be found never folds.
  Each gets the reply Tend for Mac's Dismiss sends and a `closure` (`reason:
  "subject_gone"`, the cause, the episode, `by: "person"` and the surface).
  Closing a recovery item records the dismissal in the original request's
  interruption decision, as any answer does; the request's status and
  authority are unchanged and nothing is restarted. A close is final, and
  every surface's confirmation says so before it sends anything.
  **Close all is on by default.** `[review] close_unanswerable = false`
  turns it off: the IPC and HTTP routes then refuse ("closing unanswerable
  questions is turned off; set [review] close_unanswerable = true") and
  `hello` does not advertise `close_unanswerable_v1`, so surfaces show no
  button. Folding and Keep work either way.

Both are operator actions: the IPC calls need the console origin, and the
routes are authorized like the other decision routes (an operator on the web
or a paired device). `tend review cleanup` lists these questions with id,
age, action, cause and since, and never closes one without `--questions
--apply`; `--json` adds `questions` and `unanswerable_count` to the report.
`tend review queue` shows them last, marked `✕ gone`. Each Fleet host judges
its own questions.

**On another Fleet host** (`docs/proposals/FLEET_UNANSWERABLE.md`), both
routes take `host`, the host's Fleet alias; without it, or with this host's
alias, they act here. The host's own daemon applies them under its own locks
and checks, and its own `[review] close_unanswerable` decides whether Close
all runs there. Its relay advertises `fleet_decision_unanswerable_v1` for
Keep and, only while that switch is on, `fleet_close_unanswerable_v1`. Each
closure records the person's own surface and actor, and that host's
collaboration audit records `via_fleet: {controller, actor, surface}`. One
host per Close all. In `GET /api/decisions`, a remote section carries
`subject_check` (Keep can be sent), `close_unanswerable` (Close all can be
sent) and, when either is false, `unanswerable_note` saying why: an
observe-only host (403), Close all turned off there (403, Keep still
works), or a relay that needs an upgrade (409). The raw Fleet command route
and the IPC socket refuse both; every attempt goes in the decision audit
with its `host`. The web and the phone show one Close all per host and name it
in the confirmation; Tend for Mac opens another host's question on the
dashboard (`/needs#item=<id>&host=<alias>` opens the folded group there)
and keeps and closes only its own.

Tend for Mac, the web Home and Tend for iPhone and iPad list them in a
collapsed **Can't be answered (N)** group at the end of Your turn (N is
`unanswerable_count`), out of Your turn's count, the badges and the Mac's
menu bar and Dock counts. Each row says why and since when and offers
**Keep** where the host takes it. **Close all** shows only while the host
advertises `close_unanswerable_v1` (the dashboard's `close_unanswerable`
flag; for another Fleet host, as above), asks
once ("Close N questions? Each asking agent gets a reply that you dismissed
it. This can't be undone."), sends exactly the listed pairs and says which
were skipped. Their history reads ask for closures (`include_closures`,
`?closures=true`) and show tend's own with their reason. Older daemons send
none of this, and the surfaces behave as before.

Tend for Mac draws this queue in Home's **Your turn** and at the top of the
Review sheet; see [MACOS](MACOS.md#workspace-information-architecture). It
opens commit and range evidence in a read-only Changes view on the person's
click, with the same read-only git settings as the rest of Changes
(`--no-ext-diff`, `--no-textconv`, `core.fsmonitor=false`, no optional locks,
no lazy fetch from a promisor remote, and every transport refused through
`protocol.allow=never` and an empty `GIT_ALLOW_PROTOCOL`, so an older git that
ignores `GIT_NO_LAZY_FETCH` cannot fetch either).

## Status line

`tend status-line --needs-attention` appends open review items after the
attention count:

```
⚠ 2 need you ◇ 3 to review · 4h
```

The age is the oldest open item. Both counts share the status line's tight
deadline; a daemon without review items, or with an unavailable store, adds
nothing.

## Storage

`$XDG_DATA_HOME/tend/reviews.sqlite3`, owned by tendd. The database is
created owner-only (0600), and an existing database and its WAL/SHM files
are tightened to 0600 on open. A symlinked path is refused; the 0700 data
directory, not an atomic no-follow open, is what keeps other users out.
Every request and disposition is also appended to an event log in the same
file for later flow metrics. If the file cannot be opened, tendd still
starts and refuses review requests with the reason.

Requires a tendd that advertises `review_items_v1`.
