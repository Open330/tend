import test from "node:test";
import assert from "node:assert/strict";

import { hostSections } from "../src/dashboard/web/needs-model.mjs";

const now = Date.parse("2026-11-19T12:00:00Z");

// A queue from a daemon with `subject_check_v1`: the folded question carries
// `unanswerable`, the queue `unanswerable_count`, the host section
// `subject_check` and `close_unanswerable`. A reader that does not know
// them still shows every item (old clients list it in Your turn).
test("a queue with unanswerable questions still decodes item by item", () => {
  const sections = hostSections(
    {
      hosts: [
        {
          alias: "this host",
          local: true,
          control: true,
          subject_check: true,
          close_unanswerable: false,
          covers: ["questions"],
          unavailable: [],
          queue: {
            items: [
              {
                kind: "human_request",
                request_id: "rq_live",
                body: "Merge #12?",
                from_session_id: "s1",
                from_pane: "%3",
                created_at: "2026-11-19T11:00:00Z",
              },
              {
                kind: "human_request",
                request_id: "rq_gone",
                body: "Deploy?",
                from_session_id: "s2",
                from_pane: "%42",
                created_at: "2026-11-10T11:00:00Z",
                unanswerable: { cause: "pane_gone", episode: "ep_1", since: "2026-11-16T00:00:00Z", pane: "%42" },
              },
            ],
            truncated: false,
            unanswerable_count: 1,
          },
        },
      ],
    },
    now,
  );
  assert.equal(sections.length, 1);
  const ids = sections[0].cards.map((card) => card.id).sort();
  assert.deepEqual(ids, ["rq_gone", "rq_live"]);
});
