//! The parity contract with Tend for Mac: both board builders read
//! `apps/shared/Fixtures/home-board-parity-input.json` and must produce
//! `home-board-parity.json`, a summary of every unit each lane holds, in
//! order. The Mac's half is `apps/tend-macos/Tests/HomeBoardParityTests.swift`;
//! a rule changed on one side fails there or here until the other follows.
//! `UPDATE_FIXTURES=1` rewrites the summary from this side (a deliberate
//! change, reviewed in the diff); a normal run only compares.

use super::*;
use serde_json::json;

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/shared/Fixtures")
}

fn read(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join(name)).unwrap()).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn agent_id(agent: &Value) -> String {
    format!(
        "{}:{}",
        agent["host"].as_str().unwrap_or(""),
        agent["session_id"].as_str().unwrap_or("")
    )
}

/// A title in English, as the Mac's `String(localized:)` reads in an
/// English locale: plain text as it is, the few keys a title can be by
/// their English words (home-strings.mjs says them the same way).
fn english(text: &Value) -> Value {
    let arg = |name: &str| text[name].as_str().unwrap_or("").to_owned();
    match text["key"].as_str() {
        None => text.clone(),
        Some("kind_on_host") => json!(format!("{} on {}", arg("kind"), arg("host"))),
        Some("background_task") => json!("\u{27f3} Background task"),
        Some("reply_ready") => json!(format!("Reply from {} is ready", arg("who"))),
        Some(other) => panic!("a title key the contract does not word yet: {other}"),
    }
}

/// One unit as the contract compares it.
pub(super) fn summary(item: &Value, lane: &str) -> Value {
    let focus = &item["focus"];
    json!({
        "id": item["id"],
        "lane": lane,
        "source": item["source"],
        "title": english(&item["title"]),
        "repo": item["repo"],
        "agents": item["agents"].as_array().into_iter().flatten().map(agent_id).collect::<Vec<_>>(),
        "helpers": item["helpers"].as_array().into_iter().flatten().map(agent_id).collect::<Vec<_>>(),
        "focus": if focus.is_object() { json!({"agent": agent_id(focus), "state": focus["state"]}) } else { Value::Null },
        "stages": item["stages"].as_array().into_iter().flatten()
            .map(|s| json!({"name": s["name"], "state": s["state"]}))
            .collect::<Vec<_>>(),
        "since": item["since"],
        "revision": item["revision"],
        "review_key": item["review_key"],
        "reviewed": item["reviewed"],
        "requested_by_person": item["requested_by_person"],
        "handled_limit": item["handled_limit"],
        "probable_helper": item["probable_helper"],
        "orphan_helper": item["orphan_helper"],
        "empty_session": item["empty_session"],
    })
}

fn build_case(case: &Value, now: OffsetDateTime) -> Value {
    let input = BoardInput {
        hosts: case["hosts"]
            .as_array()
            .unwrap()
            .iter()
            .map(host_from_snapshot)
            .collect(),
        runs: case["runs"].as_array().cloned().unwrap_or_default(),
        reviewed: Some(strings(&case["reviewed"]).into_iter().collect()),
        now: Some(now),
        ..BoardInput::default()
    };
    let board = build(&input);
    let mut lanes = serde_json::Map::new();
    for lane in ["needs_you", "running", "finished"] {
        let units: Vec<Value> = board["lanes"][lane]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["items"].as_array().cloned().unwrap_or_default())
            .filter(|item| item["type"] == "unit")
            .map(|item| summary(&item, lane))
            .collect();
        lanes.insert(lane.to_owned(), Value::Array(units));
    }
    json!({"name": case["name"], "lanes": lanes})
}

#[test]
fn the_board_matches_the_parity_contract_tend_for_mac_also_meets() {
    let input = read("home-board-parity-input.json");
    assert_eq!(input["schema"], 1);
    let now = OffsetDateTime::parse(input["now"].as_str().unwrap(), &Rfc3339).unwrap();
    let cases: Vec<Value> = input["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| build_case(case, now))
        .collect();
    let built = json!({"schema": 1, "cases": cases});
    // Checked before anything is written, so a regeneration cannot record
    // a summary that stopped testing what its cases are named for.
    assert_named_facts(&built);
    let path = fixtures().join("home-board-parity.json");
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&built).unwrap() + "\n").unwrap();
    }
    let recorded = read("home-board-parity.json");
    for (built, recorded) in built["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(recorded["cases"].as_array().unwrap())
    {
        assert_eq!(built, recorded, "case: {}", built["name"]);
    }
    assert_eq!(built, recorded);
}

/// Hand-checked facts, so regenerating the summary cannot bless a case that
/// stopped exercising what it is named for.
#[test]
fn the_parity_cases_exercise_what_they_are_named_for() {
    assert_named_facts(&read("home-board-parity.json"));
}

fn assert_named_facts(recorded: &Value) {
    let case = |name: &str| {
        recorded["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"].as_str().unwrap().starts_with(name))
            .unwrap_or_else(|| panic!("no case {name}"))
            .clone()
    };
    let ids = |case: &Value, lane: &str| -> Vec<String> {
        case["lanes"][lane]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let solo = case("solo agents");
    assert_eq!(ids(&solo, "running"), ["agent:mba:w"]);
    assert_eq!(ids(&solo, "needs_you"), ["agent:mba:q"]);
    assert_eq!(ids(&solo, "finished"), ["agent:mba:d"]);
    let pipeline = case("a pipeline run");
    assert_eq!(ids(&pipeline, "running"), ["work:tend/fix-login"]);
    assert_eq!(
        pipeline["lanes"]["running"][0]["stages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let helpers = case("a helper folds");
    assert_eq!(
        helpers["lanes"]["running"][0]["helpers"],
        json!(["mba:help"])
    );
    assert!(helpers["lanes"]["finished"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["orphan_helper"] == true));
    let hosts = case("the same session id");
    assert_eq!(ids(&hosts, "running"), ["agent:mini:same"]);
    assert_eq!(ids(&hosts, "finished"), ["agent:mba:same"]);
    let who = case("who asked");
    let requested: Vec<(String, bool)> = who["lanes"]["finished"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["id"].as_str().unwrap().to_owned(),
                u["requested_by_person"] == true,
            )
        })
        .collect();
    assert!(requested.contains(&("agent:mba:p".into(), true)));
    assert!(requested.contains(&("agent:mba:peer".into(), false)));
    assert!(requested.contains(&("agent:mba:note".into(), false)));
    let marks = case("a reviewed result");
    let seen = marks["lanes"]["finished"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "agent:mba:seen")
        .unwrap();
    assert_eq!(
        seen["reviewed"], true,
        "the recorded mark is this result's key"
    );
    let changed = marks["lanes"]["finished"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "agent:mba:changed")
        .unwrap();
    assert_eq!(
        changed["reviewed"], false,
        "a mark for an earlier result does not hide this one"
    );
    assert_eq!(
        ids(&marks, "needs_you").len(),
        2,
        "the uncovered cap and the question"
    );
}
