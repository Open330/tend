# Home's numbers: one meaning each

Tend for Mac, the iPhone/iPad app and the web dashboard show the same
numbers for the same board. tendd computes them once, as `counts` in
`GET /api/home` and `GET /api/device/home` (`crates/tend/src/home_board.rs`,
`counts_json`). A client shows them as given. The Mac computes its own board
and holds to the same rules. Plan: `docs/proposals/PARITY.md` P3 and P4.

| Field | What it counts | Shown as |
|---|---|---|
| `your_turn` | Your turn's cards, decisions and work units together. A question nobody can answer is not one of them. Your turn never folds, so this is every card the lane shows. | Your turn's header, **and the badge**: the Mac rail's Home badge, the web's rail badge and `(N)` tab title, the iPhone tab and the iPad sidebar. |
| `actionable` | Of `your_turn`, the decisions the viewer can decide from here. A review on a host that does not take review decisions from here is not one. A work unit waiting in its pane is never one: it is answered in the pane. | Optional. A surface may say "N to decide here", but the badge stays `your_turn`. |
| `cant_answer` | Questions nobody can answer. This is at least the number listed: a host's exact count when it reports one. | The "Can't be answered (N)" group, outside Your turn's number. |
| `in_progress` | In progress's work units, including the ones folded behind "empty sessions", "possible helpers" or "older". | In progress's header. |
| `finished` | Finished's work units not set aside with Looks Good, including the folded ones. | Not a header: the total behind the rows below. |
| `finished_shown` | Of `finished`, the main list: units whose last turn was within a day, and that are not an empty session or a possible helper. | Finished's header. It counts only what the lane shows; each fold row says its own number. |
| `finished_older` | Of `finished`, units whose last turn was one to seven days ago. | "N older". |
| `finished_stale` | Of `finished`, units idle more than seven days. | "N idle over a week". |
| `reviewed` | Finished's units set aside with Looks Good. | "Show N reviewed". |
| `coverage` | About the decision queues only. `complete` when every host has a queue read whole for every kind (questions, waiting agents, review items). `partial` when a host has no section, no queue, does not cover a kind, was cut, or could not be read. | When `partial`, a surface says so next to the number ("1 need you, maybe more") and never presents it as everything. |

A Finished unit's age is its **last turn**: the later of its agents' last
prompt and last response, else when the agent started (`age_group` on each
unit: `recent`, `older` or `stale`). It is never `state_entered_at`, which
hooks reset on agents that have done nothing for days. Each unit is in
exactly one row, folded in this order: possible helper, empty session, then
by age.

All of these are **totals before a client's own filter**. Whether Looks
Good marks were read at all is `reviewed_marks`, beside `counts`. Without
the marks, `finished` counts every finished unit and `reviewed` is 0.

These numbers are not counted, and do not belong in a header or a badge:

- **Decided history** (answered, accepted, returned). It belongs under its
  own heading.
- **Unread agents** (the Mac's Dock count) and notification counts. They
  are a different question, and each keeps its own label.
- **A local filter** (host, project) narrows what a lane shows. A filtered
  header says it is filtered. It does not quietly change the meaning of the
  number.

Tests: `crates/tend/src/home_board/tests.rs` (`counts_have_one_meaning_each`,
`counts_are_complete_when_every_queue_was_read_whole`), and the fixtures
`apps/shared/Fixtures/home-board.json` and `device-home.json`.
