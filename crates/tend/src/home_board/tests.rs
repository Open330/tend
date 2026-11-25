//! The board's rules, case by case, against Tend for Mac's `FleetBoard`
//! (apps/tend-macos/Tests/FleetHomeTests.swift covers the same ground).

use super::*;
use serde_json::json;

fn now() -> OffsetDateTime {
    OffsetDateTime::parse("2026-11-16T09:00:00Z", &Rfc3339).unwrap()
}

fn at(minutes_ago: i64) -> String {
    (now() - Duration::minutes(minutes_ago))
        .format(&Rfc3339)
        .unwrap()
}

fn agent(session: &str, pane: &str, state: &str, prompt: &str, minutes_ago: i64) -> Value {
    json!({
        "agent_session_id": session, "kind": "claude_code", "pane": pane, "tmux_socket": "s",
        "state": state, "last_prompt": prompt, "state_entered_at": at(minutes_ago),
    })
}

fn pane(id: &str, path: &str) -> Value {
    json!({"pane_id": id, "socket": "s", "window_id": "@1", "current_path": path})
}

fn host(alias: &str, local: bool, agents: Vec<Value>, panes: Vec<Value>) -> BoardHost {
    BoardHost {
        alias: alias.into(),
        local,
        state: "online".into(),
        error: None,
        mode: None,
        tend_version: None,
        agents,
        panes,
    }
}

fn board(input: BoardInput) -> Value {
    build(&BoardInput {
        now: Some(now()),
        ..input
    })
}

fn items(board: &Value, lane: &str) -> Vec<Value> {
    board["lanes"][lane]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["items"].as_array().unwrap().clone())
        .collect()
}

#[test]
fn solo_agents_take_the_lane_of_their_state() {
    let b = board(BoardInput {
        hosts: vec![host(
            "mba",
            true,
            vec![
                agent("w", "%1", "working", "port the board", 2),
                agent("q", "%2", "waiting_input", "pick a font", 5),
                json!({"agent_session_id": "d", "kind": "codex", "pane": "%3", "tmux_socket": "s", "state": "idle",
                       "last_prompt": "fix toast", "last_response": "Done: toast fixed\nmore", "state_entered_at": at(10)}),
                agent("x", "%4", "stopped", "gone", 1),
            ],
            vec![
                pane("%1", "/w/tend"),
                pane("%2", "/w/site"),
                pane("%3", "/w/tend"),
            ],
        )],
        ..Default::default()
    });
    let running = items(&b, "running");
    assert_eq!(running.len(), 1);
    assert_eq!(running[0]["title"], "port the board");
    assert_eq!(running[0]["repo"], "tend");
    assert_eq!(running[0]["id"], "agent:mba:w");
    let turn = items(&b, "needs_you");
    assert_eq!(turn[0]["id"], "agent:mba:q");
    assert_eq!(turn[0]["stages"][0]["state"], "needs_you");
    let finished = items(&b, "finished");
    assert_eq!(finished[0]["headline"], "Done: toast fixed");
    assert_eq!(finished[0]["stages"][0]["name"], "codex");
    assert_eq!(b["agent_count"], 3, "a stopped agent is not on the board");
}

#[test]
fn a_pipeline_run_groups_its_agents_into_one_unit_with_stages() {
    let mut impl_pane = pane("%1", "/w/tend");
    impl_pane["workspace_id"] = json!("tend");
    impl_pane["work_id"] = json!("fix-login");
    impl_pane["agent_alias"] = json!("impl");
    let mut review_pane = pane("%2", "/w/tend");
    review_pane["workspace_id"] = json!("tend");
    review_pane["work_id"] = json!("fix-login");
    review_pane["agent_alias"] = json!("review");
    let run = json!({
        "identity": {"workspace_id": "tend", "work_id": "fix-login"},
        "pipeline": "impl-review", "cwd": "/w/tend", "generation": 3,
        "desired": [
            {"alias": "impl", "program": "claude", "role": "Implement"},
            {"alias": "review", "program": "codex", "role": "Review"},
        ],
        "aliases": {"impl": {"alias": "impl", "status": "done", "generation": 3},
                    "review": {"alias": "review", "status": "running", "generation": 3}},
    });
    let mut reviewer = agent("r", "%2", "working", "review the diff", 1);
    reviewer["kind"] = json!("codex");
    reviewer["recap"] = json!("Reading the retry path");
    let b = board(BoardInput {
        hosts: vec![host(
            "mba",
            true,
            vec![agent("i", "%1", "idle", "implement", 20), reviewer],
            vec![impl_pane, review_pane],
        )],
        runs: vec![run],
        ..Default::default()
    });
    let running = items(&b, "running");
    assert_eq!(running.len(), 1, "one unit for the Work, not one per agent");
    let unit = &running[0];
    assert_eq!(unit["id"], "work:tend/fix-login");
    assert_eq!(unit["title"], "fix-login");
    assert_eq!(unit["stages"].as_array().unwrap().len(), 2);
    assert_eq!(unit["stages"][0]["name"], "Implement");
    assert_eq!(unit["stages"][0]["state"], "done");
    assert_eq!(unit["stages"][1]["state"], "running");
    assert_eq!(
        unit["headline"], "Reading the retry path",
        "the working agent's line"
    );
    assert_eq!(unit["agents"].as_array().unwrap().len(), 2);
    assert!(unit["revision"].as_str().unwrap().contains("gen:3"));
}

#[test]
fn a_helper_folds_into_the_unit_it_helps_and_an_orphan_stays_marked() {
    let claimed = |from: &str| {
        format!(
            "[tend] Claimed; review this.\n--- peer request ---\n{}\n--- end peer request ---",
            json!({"from": {"agent_session_id": from, "label": "claude@%1"}})
        )
    };
    let b = board(BoardInput {
        hosts: vec![host(
            "mba",
            true,
            vec![
                agent("parent", "%1", "idle", "ship the fix", 30),
                agent("helper", "%2", "working", &claimed("parent"), 1),
                agent("orphan", "%3", "working", &claimed("gone"), 2),
                agent(
                    "probable",
                    "%4",
                    "working",
                    "tend msg accept req_0123abcd_1f from %1",
                    3,
                ),
                json!({"agent_session_id": "empty", "kind": "claude_code", "pane": "%5", "state": "idle", "state_entered_at": at(1)}),
            ],
            vec![],
        )],
        ..Default::default()
    });
    let running = items(&b, "running");
    let parent = running
        .iter()
        .find(|u| u["id"] == "agent:mba:parent")
        .expect("a finished parent with a working helper is in progress");
    assert_eq!(parent["helpers"].as_array().unwrap().len(), 1);
    assert_eq!(parent["headline"]["key"], "waiting_on");
    assert!(
        !running.iter().any(|u| u["id"] == "agent:mba:helper"),
        "the helper is folded"
    );
    let orphan = running
        .iter()
        .find(|u| u["id"] == "agent:mba:orphan")
        .unwrap();
    assert_eq!(orphan["orphan_helper"], true);
    let probable = running
        .iter()
        .find(|u| u["id"] == "agent:mba:probable")
        .unwrap();
    assert_eq!(probable["probable_helper"], true);
    let empty = items(&b, "finished")
        .into_iter()
        .find(|u| u["id"] == "agent:mba:empty")
        .unwrap();
    assert_eq!(empty["empty_session"], true);
}

#[test]
fn a_covered_usage_cap_keeps_working_and_an_uncovered_one_needs_you() {
    let capped = |session: &str, kind: &str| {
        json!({"agent_session_id": session, "kind": kind, "pane": session, "state": "idle",
               "last_prompt": "long job", "rate_limit_scope": "five_hour",
               "rate_limited_until": (now() + Duration::hours(1)).format(&Rfc3339).unwrap(),
               "state_entered_at": at(3)})
    };
    let automation = json!({"enabled": true, "rules": [
        {"name": "resume-after-limit", "on": "rate_limited", "enabled": true, "action": "send_prompt", "agent": ["claude_code"]},
    ]});
    let b = board(BoardInput {
        hosts: vec![
            host(
                "mba",
                true,
                vec![capped("%1", "claude_code"), capped("%2", "codex")],
                vec![],
            ),
            host("mini", false, vec![capped("%9", "claude_code")], vec![]),
        ],
        automation: Some(automation.clone()),
        ..Default::default()
    });
    let running = items(&b, "running");
    assert_eq!(running.len(), 1);
    assert_eq!(
        running[0]["handled_limit"], true,
        "the rule resumes the local claude"
    );
    let turn: Vec<String> = items(&b, "needs_you")
        .iter()
        .map(|u| u["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        turn.contains(&"agent:mba:%2".to_owned()),
        "no rule covers codex"
    );
    assert!(
        turn.contains(&"agent:mini:%9".to_owned()),
        "another host's cap is never covered here"
    );
    // Paused automation covers nothing.
    let mut paused = automation;
    paused["paused_until"] = json!((now() + Duration::minutes(5)).format(&Rfc3339).unwrap());
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![capped("%1", "claude_code")], vec![])],
        automation: Some(paused),
        ..Default::default()
    });
    assert_eq!(items(&b, "needs_you").len(), 1);
}

#[test]
fn your_turn_interleaves_decisions_longest_wait_first_and_tends_own_last() {
    let decisions = vec![json!({
        "alias": "mba", "local": true, "control": true, "decides_reviews": true,
        "queue": {"items": [
            {"kind": "human_request", "request_id": "sys", "body": "recover", "system": true, "created_at": at(90)},
            {"kind": "human_request", "request_id": "new", "body": "newer", "created_at": at(1), "from_session_id": "q"},
            {"kind": "review", "id": "rv", "summary": "result", "requested_at": at(40)},
            {"kind": "agent_waiting", "session_id": "q", "state": "waiting_input", "since": at(5)},
        ]},
    })];
    let mut asker = agent("q", "%2", "waiting_input", "pick", 5);
    asker["cwd"] = json!("/w/site");
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![asker], vec![])],
        decisions,
        ..Default::default()
    });
    let order: Vec<String> = items(&b, "needs_you")
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        order.len(),
        4,
        "the waiting agent is a unit, listed once: {order:?}"
    );
    assert_eq!(order.last().unwrap(), "human:mba:sys");
    let groups = b["lanes"]["needs_you"]["groups"].as_array().unwrap();
    assert_eq!(groups.last().unwrap()["system"], true);
    let site = groups.iter().find(|g| g["project"] == "site").unwrap();
    assert_eq!(
        site["items"].as_array().unwrap().len(),
        2,
        "the question takes its asker's project"
    );
    assert_eq!(b["lanes"]["needs_you"]["count"], 4);
}

#[test]
fn one_review_id_on_two_hosts_is_two_items() {
    let section = |alias: &str, local: bool| {
        json!({
            "alias": alias, "local": local, "control": true, "decides_reviews": true,
            "queue": {"items": [{"kind": "review", "id": "rv_same", "summary": alias, "requested_at": at(10)}]},
        })
    };
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![], vec![])],
        decisions: vec![
            section("mba", true),
            section("mini", false),
            section("pc", false),
        ],
        ..Default::default()
    });
    let mut ids: Vec<String> = items(&b, "needs_you")
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    // This host's keeps the Mac's key; another host's names that host.
    assert_eq!(
        ids,
        ["review:mini:rv_same", "review:pc:rv_same", "review:rv_same"]
    );
}

#[test]
fn reviewed_marks_use_the_macs_key() {
    // FleetReview.reviewKey: "home:" + hex(SHA-256("<id>\n<revision>")).
    assert_eq!(
        review_key("agent:mba:d", "r"),
        format!("home:{}", hex_of(&Sha256::digest(b"agent:mba:d\nr")))
    );
    let finished = json!({"agent_session_id": "d", "kind": "claude_code", "pane": "%3", "state": "idle",
                           "last_prompt": "fix toast", "last_response": "done", "state_entered_at": at(10)});
    let first = board(BoardInput {
        hosts: vec![host("mba", true, vec![finished.clone()], vec![])],
        ..Default::default()
    });
    let key = items(&first, "finished")[0]["review_key"]
        .as_str()
        .unwrap()
        .to_owned();
    let reviewed = board(BoardInput {
        hosts: vec![host("mba", true, vec![finished], vec![])],
        reviewed: Some(HashSet::from([key])),
        ..Default::default()
    });
    assert_eq!(items(&reviewed, "finished")[0]["reviewed"], true);
    assert_eq!(items(&reviewed, "finished")[0]["requested_by_person"], true);
}

fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[test]
fn prompt_titles_read_tends_notices_as_what_they_are() {
    assert_eq!(
        prompt_title("<task-notification><summary>Build finished</summary></task-notification>"),
        Some(Text::plain("⟳ Build finished"))
    );
    assert_eq!(
        prompt_title("<task-notification>x"),
        Some(Text::key("background_task"))
    );
    assert_eq!(
        prompt_title("[tend:reply] Completed reply from codex@%13 (abc) is ready. Read it"),
        Some(Text::with(
            "reply_ready",
            vec![("who", Text::plain("codex@%13"))]
        ))
    );
    assert_eq!(
        prompt_title("[tend:review] Review from codex@%39. Read one"),
        Some(Text::plain("Review from codex@%39"))
    );
    assert_eq!(prompt_title("  plain\nsecond"), Some(Text::plain("plain")));
}

#[test]
fn the_backlog_lists_queued_work_that_is_not_running() {
    let b = board(BoardInput {
        backlog: vec![
            json!({"identity": {"workspace_id": "tend", "work_id": "next"}, "metadata": {"stage": "queued", "goal": "Do next"},
                   "plan": {"cwd": "/w/tend", "priority": 2}}),
            json!({"identity": {"workspace_id": "tend", "work_id": "done"}, "metadata": {"stage": "done"}, "plan": {}}),
        ],
        ..Default::default()
    });
    let backlog = b["backlog"].as_array().unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0]["title"], "next");
    assert_eq!(backlog[0]["line"], "Do next");
    assert_eq!(backlog[0]["project"], "tend");
}

#[test]
fn first_line_caps_at_160_characters() {
    let long = "x".repeat(200);
    let line = first_line(&long).unwrap();
    assert_eq!(line.chars().count(), 160);
    assert!(line.ends_with('…'));
    assert_eq!(first_line("\n  \n hello \n"), Some("hello".into()));
}

#[test]
fn a_unicode_prompt_title_never_panics() {
    assert_eq!(
        prompt_title("[tend:req_abcd] İreply from é"),
        Some(Text::with("reply_ready", vec![("who", Text::plain("é"))]))
    );
    assert_eq!(
        prompt_title("[tend:x] Ünïcode only."),
        Some(Text::plain("Ünïcode only"))
    );
    assert_eq!(
        prompt_title("[tend:x] REPLY FROM codex@%2 is ready"),
        Some(Text::with(
            "reply_ready",
            vec![("who", Text::plain("codex@%2"))]
        ))
    );
}

fn run(desired: Value, aliases: Value) -> Value {
    json!({
        "identity": {"workspace_id": "tend", "work_id": "w"}, "pipeline": "p", "cwd": "/w/tend", "generation": 1,
        "desired": desired, "aliases": aliases,
    })
}

fn stamped(id: &str, alias: &str) -> Value {
    let mut p = pane(id, "/w/tend");
    p["workspace_id"] = json!("tend");
    p["work_id"] = json!("w");
    p["agent_alias"] = json!(alias);
    p
}

#[test]
fn a_blocked_stage_with_nobody_on_it_has_no_focus() {
    let b = board(BoardInput {
        hosts: vec![host(
            "mba",
            true,
            vec![agent("i", "%1", "working", "impl", 1)],
            vec![stamped("%1", "impl")],
        )],
        runs: vec![run(
            json!([{"alias": "impl", "program": "claude"}, {"alias": "review", "program": "codex"}]),
            json!({"impl": {"alias": "impl", "status": "running"}, "review": {"alias": "review", "status": "blocked", "error": "review crashed"}}),
        )],
        ..Default::default()
    });
    let unit = &items(&b, "needs_you")[0];
    assert_eq!(unit["headline"], "review crashed");
    assert!(unit["focus"].is_null(), "not impl's pane: {unit}");
    // The planned Codex stage's mark shows, though no Codex agent runs yet.
    assert_eq!(unit["agent_kinds"], json!(["claude_code", "codex"]));
}

#[test]
fn a_reviews_target_session_names_its_project() {
    let mut asker = agent("s1", "%2", "idle", "fix", 5);
    asker["cwd"] = json!("/w/site");
    let decisions = vec![json!({"alias": "mba", "local": true, "queue": {"items": [
        {"kind": "review", "id": "rv", "summary": "r", "requested_at": at(4), "target": {"kind": "session", "session_id": "s1"}},
    ]}})];
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![asker], vec![])],
        decisions,
        ..Default::default()
    });
    let review = items(&b, "needs_you")
        .into_iter()
        .find(|i| i["type"] == "decision")
        .unwrap();
    assert_eq!(review["repo"], "site");
}

#[test]
fn a_label_only_helper_with_no_resolved_pane_stays_an_orphan() {
    let b = board(BoardInput {
        hosts: vec![host(
            "mba",
            true,
            vec![
                agent("a", "%1", "working", "parent work", 2),
                agent(
                    "b",
                    "%2",
                    "working",
                    "[tend:req_abcd_1] Review from codex@%1.",
                    1,
                ),
            ],
            vec![],
        )],
        ..Default::default()
    });
    let running = items(&b, "running");
    assert_eq!(running.len(), 2, "no pane scan, no folding by label");
    assert!(running.iter().any(|u| u["orphan_helper"] == true));
}

#[test]
fn decision_sources_that_could_not_be_read_are_reported() {
    let decisions = vec![
        json!({"alias": "mba", "local": true, "unavailable": ["review database failed"], "queue": {"items": [], "truncated": true}}),
    ];
    let b = board(BoardInput {
        decisions,
        ..Default::default()
    });
    assert_eq!(
        b["decision_hosts"][0]["unavailable"][0],
        "review database failed"
    );
    assert_eq!(b["decision_hosts"][0]["truncated"], true);
}

#[test]
fn unanswerable_questions_stay_listed_but_leave_your_turns_count() {
    let decisions = vec![json!({
        "alias": "mba", "local": true, "control": true, "decides_reviews": true,
        "subject_check": true, "close_unanswerable": false,
        "queue": {"items": [
            {"kind": "human_request", "request_id": "live", "body": "merge?", "created_at": at(30), "from_session_id": "a"},
            {"kind": "human_request", "request_id": "gone", "body": "deploy?", "created_at": at(3000), "from_session_id": "b",
             "unanswerable": {"cause": "pane_gone", "episode": "ep1", "since": at(1600), "pane": "%9"}},
        ], "truncated": true, "unanswerable_count": 3},
    })];
    let b = board(BoardInput {
        decisions,
        ..Default::default()
    });
    let listed = items(&b, "needs_you");
    assert_eq!(listed.len(), 2, "older clients still see both: {listed:?}");
    assert_eq!(b["lanes"]["needs_you"]["count"], 1);
    let host = &b["decision_hosts"][0];
    assert_eq!(host["subject_check"], true);
    assert_eq!(host["close_unanswerable"], false);
    assert_eq!(host["unanswerable_count"], 3);
}

#[test]
fn a_long_helper_chain_resolves_to_its_root() {
    let claimed = |from: &str| {
        format!(
            "[tend] Claimed; x\n--- peer request ---\n{}\n--- end peer request ---",
            json!({"from": {"agent_session_id": from}})
        )
    };
    let mut agents = vec![agent("s0", "%0", "idle", "root work", 100)];
    for i in 1..200 {
        agents.push(agent(
            &format!("s{i}"),
            &format!("%{i}"),
            "idle",
            &claimed(&format!("s{}", i - 1)),
            100 - i / 2,
        ));
    }
    let b = board(BoardInput {
        hosts: vec![host("mba", true, agents, vec![])],
        ..Default::default()
    });
    let finished = items(&b, "finished");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["helpers"].as_array().unwrap().len(), 199);
}

#[test]
fn a_long_result_is_cut_on_a_character_with_an_ellipsis() {
    assert_eq!(cap_chars("short", 10), ("short".to_owned(), false));
    let (cut, was_cut) = cap_chars("가나다라마바", 3);
    assert!(was_cut);
    assert_eq!(cut, "가나다\u{2026}");
}

#[test]
fn the_asked_digest_reads_markdown_as_the_mac_does() {
    assert_eq!(
        digest("# Fix it\n\n- one\n---\n`two`", 600).as_deref(),
        Some("Fix it one two`")
    );
    assert_eq!(digest("  \n---\n", 600), None);
    let long = format!("{}. {}", "a".repeat(40), "b ".repeat(40));
    assert_eq!(
        digest(&long, 50).as_deref(),
        Some(format!("{}.", "a".repeat(40)).as_str())
    );
    assert_eq!(digest("word word word", 9).as_deref(), Some("word\u{2026}"));
}

/// apps/shared/Design/COUNTS.md: one meaning per number, the same on
/// every client.
#[test]
fn counts_have_one_meaning_each() {
    let finished = |session: &str| {
        json!({"agent_session_id": session, "kind": "claude_code", "pane": "%3", "state": "idle",
               "last_prompt": "fix toast", "last_response": "done", "state_entered_at": at(10)})
    };
    let decisions = vec![
        json!({
            "alias": "mba", "local": true, "control": true, "decides_reviews": true,
            "subject_check": true,
            "queue": {"items": [
                {"kind": "human_request", "request_id": "live", "body": "merge?", "created_at": at(30), "from_session_id": "a"},
                {"kind": "human_request", "request_id": "gone", "body": "deploy?", "created_at": at(3000), "from_session_id": "b",
                 "unanswerable": {"cause": "pane_gone", "episode": "ep1", "since": at(1600), "pane": "%9"}},
            ], "truncated": true, "unanswerable_count": 3},
        }),
        // Another host that does not take review decisions from here.
        json!({
            "alias": "mini", "local": false, "control": true, "decides_reviews": false,
            "queue": {"items": [
                {"kind": "review", "id": "rv", "summary": "ship?", "requested_at": at(20), "target": {"kind": "session", "session_id": "x"}},
            ]},
        }),
    ];
    let hosts = vec![host(
        "mba",
        true,
        vec![
            agent("w", "%1", "working", "port", 5),
            finished("d1"),
            finished("d2"),
        ],
        vec![],
    )];
    let first = board(BoardInput {
        hosts: hosts.clone(),
        decisions: decisions.clone(),
        ..Default::default()
    });
    let key = items(&first, "finished")[0]["review_key"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = board(BoardInput {
        hosts,
        decisions,
        reviewed: Some(HashSet::from([key])),
        ..Default::default()
    });
    let counts = &b["counts"];
    // Your turn: the live question and the other host's review, not the
    // question nobody can answer. The badge says this number.
    assert_eq!(counts["your_turn"], 2, "{counts}");
    assert_eq!(counts["your_turn"], b["lanes"]["needs_you"]["count"]);
    // The other host's review is listed but not decidable from here.
    assert_eq!(counts["actionable"], 1);
    // One listed, three by the host's exact count.
    assert_eq!(counts["cant_answer"], 3);
    assert_eq!(counts["in_progress"], 1);
    // Two finished, one set aside with Looks Good.
    assert_eq!(counts["finished"], 1);
    assert_eq!(counts["reviewed"], 1);
    // mba's queue was cut: the numbers are not all there is.
    assert_eq!(counts["coverage"], "partial");
}

#[test]
fn counts_are_complete_when_every_queue_was_read_whole() {
    let whole = |alias: &str| {
        json!({
            "alias": alias, "local": alias == "mba", "control": true,
            "covers": ["questions", "waiting agents", "review items"],
            "queue": {"items": [], "truncated": false}, "unavailable": [],
        })
    };
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![], vec![])],
        decisions: vec![whole("mba")],
        ..Default::default()
    });
    assert_eq!(b["counts"]["coverage"], "complete");
    assert_eq!(b["counts"]["your_turn"], 0);
    // A host with no section at all: not an empty Your turn.
    let missing = board(BoardInput {
        hosts: vec![
            host("mba", true, vec![], vec![]),
            host("mini", false, vec![], vec![]),
        ],
        decisions: vec![whole("mba")],
        ..Default::default()
    });
    assert_eq!(missing["counts"]["coverage"], "partial");
    // A section that does not cover review items, with no reason given.
    let mut narrow = whole("mba");
    narrow["covers"] = json!(["questions", "waiting agents"]);
    let narrow = board(BoardInput {
        decisions: vec![narrow],
        ..Default::default()
    });
    assert_eq!(narrow["counts"]["coverage"], "partial");
    // A section with no queue.
    let mut empty = whole("mba");
    empty.as_object_mut().unwrap().remove("queue");
    let empty = board(BoardInput {
        decisions: vec![empty],
        ..Default::default()
    });
    assert_eq!(empty["counts"]["coverage"], "partial");
    let unread = board(BoardInput {
        decisions: vec![json!({
            "alias": "mini", "local": false, "covers": [],
            "unavailable": ["host is offline"],
        })],
        ..Default::default()
    });
    assert_eq!(unread["counts"]["coverage"], "partial");
}

#[test]
fn a_unit_in_your_turn_counts_for_the_badge_but_is_not_decidable_here() {
    let waiting = json!({"agent_session_id": "q", "kind": "claude_code", "pane": "%2",
                         "state": "waiting_choice", "last_prompt": "pick a font",
                         "last_notification": "Which font?", "state_entered_at": at(4)});
    let b = board(BoardInput {
        hosts: vec![host("mba", true, vec![waiting], vec![])],
        ..Default::default()
    });
    assert_eq!(items(&b, "needs_you").len(), 1);
    assert_eq!(b["counts"]["your_turn"], 1);
    assert_eq!(
        b["counts"]["actionable"], 0,
        "answered in its pane, not from a card here"
    );
}

#[test]
fn cant_answer_takes_each_hosts_larger_of_listed_and_exact() {
    let gone = |id: &str| {
        json!({"kind": "human_request", "request_id": id, "body": "?", "created_at": at(3000), "from_session_id": id,
               "unanswerable": {"cause": "ended", "episode": format!("ep_{id}"), "since": at(1600)}})
    };
    let section = |alias: &str, listed: Vec<Value>, exact: Option<u64>| {
        let mut queue = json!({"items": listed});
        if let Some(n) = exact {
            queue["unanswerable_count"] = json!(n);
        }
        json!({"alias": alias, "local": alias == "a", "control": true, "subject_check": true, "queue": queue})
    };
    let b = board(BoardInput {
        decisions: vec![
            // Lists two, counts one: two.
            section("a", vec![gone("a1"), gone("a2")], Some(1)),
            // Lists one, counts four: four.
            section("b", vec![gone("b1")], Some(4)),
            // Lists one, no count: one.
            section("c", vec![gone("c1")], None),
        ],
        ..Default::default()
    });
    assert_eq!(b["counts"]["cant_answer"], 7);
}
