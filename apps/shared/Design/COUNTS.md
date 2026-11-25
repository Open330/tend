# Home's numbers: one meaning each

Tend for Mac, the iPhone/iPad app and the web dashboard show the same
numbers for the same board. tendd computes them once, as `counts` in
`GET /api/home` and `GET /api/device/home` (`crates/tend/src/home_board.rs`,
`counts_json`). A client shows them as given. The Mac computes its own board
and holds to the same rules. Plan: `docs/proposals/PARITY.md` P3 and P4.

| Field | What it counts | Shown as |
|---|---|---|
| `your_turn` | Your turn's cards, decisions and work units together. A question nobody can answer is not one of them. Your turn never folds, so this is every card the lane shows. | Your turn's header, **and the badge**: the Mac rail's Home badge, the web's rail badge and `(N)` tab title, the iPhone tab and the iPad sidebar. |
| `actionable` | Of `your_turn`, the ones the viewer can decide from here. A review on a host that does not take review decisions from here is not one. | Optional. A surface may say "N to decide here", but the badge stays `your_turn`. |
| `cant_answer` | Questions nobody can answer. This is at least the number listed: a host's exact count when it reports one. | The "Can't be answered (N)" group, outside Your turn's number. |
| `in_progress` | In progress's work units, including the ones folded behind "empty sessions", "possible helpers" or "older". | In progress's header. |
| `finished` | Finished's work units not set aside with Looks Good, including the folded ones. | Finished's header. |
| `reviewed` | Finished's units set aside with Looks Good. | "Show N reviewed". |
| `coverage` | `complete`, or `partial` when a host's queue could not be read or was cut. | When `partial`, a surface says so next to the number ("1 need you, maybe more") and never presents it as everything. |

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
