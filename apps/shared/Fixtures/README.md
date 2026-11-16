# Decision fixtures

Recorded from a scratch tendd (`apps/tend-ios/Scripts/scratch-tendd.sh`) on
2026-11-15, so the Swift clients decode what the daemon really sends:

- `decisions.json`: `GET /api/decisions` (a question and two results).
- `decisions-history.json`: `GET /api/decisions/history` (answers and
  dispositions, one decided by a paired device).
- `decisions-fleet.json`: `decisions.json` plus, by hand, an item kind no
  client knows, an offline Fleet host, and an observe-only host with an
  agent waiting in its pane.

The dashboard's per-host `queue` and `history` are the daemon's
`decision_queue` / `decision_history` documents, the ones Tend for Mac reads
over IPC, so these also cover the shared models in `../Sources`. Re-record
them when those documents change.
- `device-fleet.json`: `GET /api/device/fleet` for an `operate` device
  (2026-11-16): one host with agents at work, finished and waiting, from
  `scratch-tendd.sh seed` and `seed-home`. Folder paths rewritten to
  `/Users/dev/work/…`. The web Home's lane rules (`home-model.mjs`) and the
  iOS app's (`Fleet.swift`) both read this shape.

- `home-board.json`: `GET /api/home`, Home's board as tendd computes it
  (recorded from the same scratch tendd on 2026-11-16, paths replaced).
- `device-home.json`: `GET /api/device/home`, the same board as an
  `operate` device gets it (no pane ids or sockets, decisions as queue
  items, units as `kind: unit`, the history in `decided`). Generated from
  the two files above by a Rust test
  (`the_device_board_fixture_is_the_projection_of_the_board_fixture`,
  `UPDATE_FIXTURES=1` rewrites it).
