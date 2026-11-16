//! Home's board, computed once in tendd for every client.
//!
//! This is Tend for Mac's `FleetBoard` (apps/tend-macos/Sources/
//! FleetHome.swift, WorkModels.swift, AgentStatus.swift, FleetReview.swift)
//! ported rule for rule, so the dashboard, the Windows app and the iOS app
//! draw the same lanes the Mac does: agents grouped into Work units with
//! stages, helper sessions folded into the unit they help, the Mac's titles
//! and headlines, rate-limit coverage, the reviewed marks Home's "Looks
//! Good" records, and the decision queue interleaved into Your turn.
//!
//! The board works on the JSON documents the Mac decodes (the Fleet
//! snapshot's agents and panes, the pipeline runs, the automation rules,
//! the swap health, the decision queue sections), not on Rust types, so a
//! remote host's agents and this host's are read the same way and the port
//! can be checked line by line against the Swift.
//!
//! It is pure: everything it reads is in [`BoardInput`], including `now`.
//! Words the client owns (status labels, the fixed headlines) are returned
//! as keys ([`Text::Key`]) so each client says them in its own language.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

/// Finished units changed within this long stay in the lane's main list.
pub const RECENT_FINISH: Duration = Duration::hours(24);

/// Capability a daemon advertises once it answers the board read.
pub const HOME_BOARD_CAPABILITY: &str = "home_board_v1";

// ---------------------------------------------------------------- input --

/// One host's agents and panes, as the Fleet snapshot carries them.
#[derive(Debug, Clone)]
pub struct BoardHost {
    pub alias: String,
    pub local: bool,
    pub state: String,
    pub error: Option<String>,
    pub agents: Vec<Value>,
    pub panes: Vec<Value>,
}

/// What the board is built from.
#[derive(Debug, Clone, Default)]
pub struct BoardInput {
    pub hosts: Vec<BoardHost>,
    /// This daemon's pipeline runs (`pipeline_runs`).
    pub runs: Vec<Value>,
    /// The decision queue, per host, as `GET /api/decisions` returns it.
    pub decisions: Vec<Value>,
    /// This daemon's automation rules (`automation_list`), for limit
    /// coverage; `None` until read (then nothing is covered).
    pub automation: Option<Value>,
    /// `swap_health.readiness == "ready"`; `None` (unknown) counts as ready.
    pub swap_ready: Option<bool>,
    /// Idempotency keys of Home's accepted review records (`home:` keys).
    /// `None` when the review store could not be read.
    pub reviewed: Option<HashSet<String>>,
    /// Work records queued in the backlog (`work_backlog`, this host).
    pub backlog: Vec<Value>,
    pub now: Option<OffsetDateTime>,
}

// ---------------------------------------------------------------- text --

/// Text a client shows: an agent's own words, or a key for words the
/// client owns, with its parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Key {
        key: &'static str,
        #[serde(flatten)]
        args: BTreeMap<&'static str, Text>,
    },
}

impl Text {
    fn key(key: &'static str) -> Self {
        Self::Key {
            key,
            args: BTreeMap::new(),
        }
    }
    fn with(key: &'static str, args: Vec<(&'static str, Text)>) -> Self {
        Self::Key {
            key,
            args: args.into_iter().collect(),
        }
    }
    fn plain(text: impl Into<String>) -> Self {
        Self::Plain(text.into())
    }
}

// ------------------------------------------------------------- helpers --

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A field's text when it holds something besides whitespace.
fn non_empty<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    str_of(value, key).map(str::trim).filter(|s| !s.is_empty())
}

/// The first non-blank line, trimmed, at most 160 characters
/// (`FleetBoard.firstLine`).
pub fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let count = line.chars().count();
    if count > 160 {
        let cut: String = line.chars().take(159).collect();
        Some(format!("{cut}…"))
    } else {
        Some(line.to_owned())
    }
}

fn line_of(value: &Value, key: &str) -> Option<String> {
    str_of(value, key).and_then(first_line)
}

fn parse_time(text: Option<&str>) -> Option<OffsetDateTime> {
    text.and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok())
}

fn format_time(at: Option<OffsetDateTime>) -> Option<String> {
    at.and_then(|t| t.format(&Rfc3339).ok())
}

/// `lastPathComponent` of a path, after one trailing slash.
pub fn repo_name(path: &str) -> Option<String> {
    let trimmed = path.trim();
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    let last = trimmed.rsplit(['/', '\\']).next()?.trim();
    (!last.is_empty()).then(|| last.to_owned())
}

/// The program name a card shows for an agent kind.
pub fn kind_name(kind: &str) -> String {
    match kind {
        "claude_code" | "claude" => "claude".into(),
        "codex" => "codex".into(),
        other => other.replace('_', " "),
    }
}

/// Which mark a kind wears; kinds of one program count once.
fn program_of(kind: &str) -> String {
    let lower = kind.to_ascii_lowercase().replace('-', "_");
    if lower.starts_with("claude") {
        "claude".into()
    } else if lower.starts_with("codex") {
        "codex".into()
    } else {
        lower
    }
}

// -------------------------------------------------------------- status --

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Error,
    NeedsInput(String),
    Asked(String),
    Limited { covered: bool },
    Working,
    Idle,
    Done,
    Other(String),
}

impl Status {
    fn needs_attention(&self) -> bool {
        match self {
            Status::Error | Status::NeedsInput(_) | Status::Asked(_) => true,
            Status::Limited { covered } => !covered,
            _ => false,
        }
    }
    fn priority(&self) -> u8 {
        match self {
            Status::Error => 0,
            Status::NeedsInput(_) => 1,
            Status::Asked(_) => 2,
            Status::Limited { .. } => 3,
            Status::Working => 4,
            Status::Idle => 5,
            Status::Done => 6,
            Status::Other(_) => 7,
        }
    }
    /// The status as a label key the client words.
    fn label(&self, agent: &Value) -> Text {
        let state = str_of(agent, "state").unwrap_or("");
        match self {
            Status::Limited { .. } => Text::with(
                "status",
                vec![
                    ("status", Text::plain("limited")),
                    (
                        "until",
                        Text::plain(str_of(agent, "rate_limited_until").unwrap_or("")),
                    ),
                ],
            ),
            Status::Asked(_) => Text::with("status", vec![("status", Text::plain("asked"))]),
            _ => Text::with("status", vec![("status", Text::plain(state))]),
        }
    }
}

/// Which of the daemon's automation rules cover an agent at a usage cap
/// (`TendLimitCoverage`): only this host's agents, only while not paused,
/// only rules with no narrower target than an agent kind and a cap scope.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    grants: Vec<Grant>,
    paused_until: Option<OffsetDateTime>,
    swap_ready: bool,
    local_sessions: HashSet<String>,
}

#[derive(Debug, Clone)]
struct Grant {
    switch: bool,
    kinds: Option<Vec<String>>,
    scopes: Option<Vec<String>>,
}

fn string_list(value: Option<&Value>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    (!list.is_empty()).then_some(list)
}

impl Coverage {
    fn new(input: &BoardInput) -> Self {
        let mut coverage = Coverage {
            swap_ready: input.swap_ready.unwrap_or(true),
            ..Default::default()
        };
        for host in input.hosts.iter().filter(|h| h.local) {
            for agent in &host.agents {
                if let Some(id) = str_of(agent, "agent_session_id") {
                    coverage.local_sessions.insert(id.to_owned());
                }
            }
        }
        let Some(rules) = &input.automation else {
            return coverage;
        };
        coverage.paused_until = parse_time(str_of(rules, "paused_until"));
        if rules.get("enabled").and_then(Value::as_bool) == Some(false) {
            return coverage;
        }
        for rule in rules
            .get("rules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let enabled = rule.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let on = str_of(rule, "on").unwrap_or("");
            let action = str_of(rule, "action").unwrap_or("");
            let narrowed = ["workspace", "work", "pane", "host", "notification"]
                .iter()
                .any(|key| rule.get(*key).is_some_and(|v| !v.is_null()));
            if !enabled
                || on != "rate_limited"
                || narrowed
                || !matches!(action, "swap_account" | "send_prompt")
            {
                continue;
            }
            let switch = action == "swap_account";
            let mut kinds = string_list(rule.get("agent"));
            if switch {
                let swappable = ["claude_code", "codex"];
                let narrowed: Vec<String> = match kinds {
                    Some(list) => list
                        .into_iter()
                        .filter(|k| swappable.contains(&k.as_str()))
                        .collect(),
                    None => swappable.iter().map(|k| (*k).to_owned()).collect(),
                };
                if narrowed.is_empty() {
                    continue;
                }
                kinds = Some(narrowed);
            }
            coverage.grants.push(Grant {
                switch,
                kinds,
                scopes: string_list(rule.get("scope")),
            });
        }
        coverage
    }

    fn covers(&self, agent: &Value, now: OffsetDateTime) -> bool {
        let Some(session) = str_of(agent, "agent_session_id") else {
            return false;
        };
        if !self.local_sessions.contains(session) {
            return false;
        }
        if self.paused_until.is_some_and(|until| until > now) {
            return false;
        }
        let kind = str_of(agent, "kind").unwrap_or("");
        let scope = str_of(agent, "rate_limit_scope").unwrap_or("");
        self.grants.iter().any(|grant| {
            (!grant.switch || self.swap_ready)
                && grant
                    .kinds
                    .as_ref()
                    .is_none_or(|k| k.iter().any(|x| x == kind))
                && grant
                    .scopes
                    .as_ref()
                    .is_none_or(|s| s.iter().any(|x| x == scope))
        })
    }
}

/// A usage cap still in force (`TendRateLimitCap.current`).
fn current_cap(agent: &Value, now: OffsetDateTime) -> bool {
    if str_of(agent, "rate_limit_scope").is_none() {
        return false;
    }
    match parse_time(str_of(agent, "rate_limited_until")) {
        Some(until) => until > now,
        None => true,
    }
}

fn status_of(agent: &Value, coverage: &Coverage, now: OffsetDateTime) -> Status {
    let state = str_of(agent, "state").unwrap_or("");
    if matches!(state, "error" | "failed" | "idle") && current_cap(agent, now) {
        return Status::Limited {
            covered: coverage.covers(agent, now),
        };
    }
    if state == "idle" {
        if let Some(question) = agent
            .get("awaiting_reply")
            .and_then(|a| non_empty(a, "question"))
        {
            return Status::Asked(question.to_owned());
        }
    }
    match state {
        "error" | "failed" => Status::Error,
        "waiting_input" | "waiting_choice" | "blocked" => Status::NeedsInput(state.to_owned()),
        "working" | "running" | "starting" => Status::Working,
        "idle" => Status::Idle,
        "done" => Status::Done,
        other => Status::Other(other.to_owned()),
    }
}

// --------------------------------------------------------------- stages --

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    NeedsYou,
    Running,
    Done,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    NeedsYou,
    Running,
    Finished,
}

fn stage_state(
    alias_status: Option<&str>,
    status: Option<&Status>,
    agent_state: Option<&str>,
) -> StageState {
    if let Some(Status::Limited { covered: true }) = status {
        return if alias_status == Some("done") {
            StageState::Done
        } else {
            StageState::Running
        };
    }
    if matches!(alias_status, Some("blocked" | "failed")) {
        return StageState::NeedsYou;
    }
    if status.is_some_and(Status::needs_attention) {
        return StageState::NeedsYou;
    }
    if alias_status == Some("done") {
        return StageState::Done;
    }
    match status {
        Some(Status::Working | Status::Limited { .. }) => return StageState::Running,
        Some(Status::Done) => return StageState::Done,
        _ => {}
    }
    if matches!(alias_status, Some("running" | "working" | "starting")) {
        return StageState::Running;
    }
    if agent_state == Some("idle") {
        return StageState::Done;
    }
    StageState::Waiting
}

fn lane_for(stages: &[Stage]) -> Lane {
    if stages.iter().any(|s| s.state == StageState::NeedsYou) {
        Lane::NeedsYou
    } else if stages.iter().all(|s| s.state == StageState::Done) {
        Lane::Finished
    } else {
        Lane::Running
    }
}

// --------------------------------------------------------------- agents --

/// An agent on a host, with its pane when the host lists exactly one.
#[derive(Debug, Clone)]
struct Hosted<'a> {
    id: String,
    host: &'a BoardHost,
    agent: &'a Value,
    pane: Option<&'a Value>,
    status: Status,
}

impl Hosted<'_> {
    fn session(&self) -> &str {
        str_of(self.agent, "agent_session_id").unwrap_or("")
    }
    fn kind(&self) -> &str {
        str_of(self.agent, "kind").unwrap_or("agent")
    }
    fn socket(&self) -> Option<&str> {
        self.pane
            .and_then(|p| str_of(p, "socket"))
            .or_else(|| str_of(self.agent, "tmux_socket"))
    }
    fn pane_id(&self) -> Option<&str> {
        self.pane
            .and_then(|p| str_of(p, "pane_id"))
            .or_else(|| str_of(self.agent, "pane"))
    }
    fn owner(&self) -> String {
        format!("{} · {}", kind_name(self.kind()), self.host.alias)
    }
    fn work_identity(&self) -> Option<(String, String)> {
        let pane = self.pane?;
        Some((
            non_empty(pane, "workspace_id")?.to_owned(),
            non_empty(pane, "work_id")?.to_owned(),
        ))
    }
    fn time(&self) -> Option<OffsetDateTime> {
        parse_time(
            str_of(self.agent, "state_entered_at")
                .or_else(|| str_of(self.agent, "last_activity_at")),
        )
    }
    fn reference(&self) -> Value {
        json!({
            "id": self.id,
            "host": self.host.alias,
            "local": self.host.local,
            "session_id": self.session(),
            "kind": self.kind(),
            "name": kind_name(self.kind()),
            "owner": self.owner(),
            "state": str_of(self.agent, "state"),
            "state_entered_at": str_of(self.agent, "state_entered_at"),
            "status": self.status.label(self.agent),
            "needs_attention": self.status.needs_attention(),
            "pane": self.pane_id(),
            "socket": self.socket(),
        })
    }
}

fn hosted_agents<'a>(
    input: &'a BoardInput,
    coverage: &Coverage,
    now: OffsetDateTime,
) -> Vec<Hosted<'a>> {
    let mut out = Vec::new();
    for host in &input.hosts {
        // Panes by id, once per host, not one scan per agent.
        let mut by_id: HashMap<&str, Vec<&Value>> = HashMap::new();
        for pane in &host.panes {
            if let Some(id) = str_of(pane, "pane_id") {
                by_id.entry(id).or_default().push(pane);
            }
        }
        for agent in &host.agents {
            if str_of(agent, "state") == Some("stopped") {
                continue;
            }
            let Some(session) = str_of(agent, "agent_session_id") else {
                continue;
            };
            let pane = str_of(agent, "pane").and_then(|pane_id| {
                let socket = str_of(agent, "tmux_socket");
                let same: &[&Value] = by_id.get(pane_id).map_or(&[], Vec::as_slice);
                same.iter()
                    .find(|p| socket.is_some() && str_of(p, "socket") == socket)
                    .copied()
                    .or_else(|| (same.len() == 1).then(|| same[0]))
            });
            out.push(Hosted {
                id: format!("{}:{session}", host.alias),
                host,
                agent,
                pane,
                status: status_of(agent, coverage, now),
            });
        }
    }
    // Status first, local host first, then by host and session
    // (AppModel.rebuildWorkspaceProjections): this order picks the
    // participants' and helpers' order and every `.first` below.
    out.sort_by(|a, b| {
        a.status
            .priority()
            .cmp(&b.status.priority())
            .then(b.host.local.cmp(&a.host.local))
            .then(a.host.alias.cmp(&b.host.alias))
            .then(a.session().cmp(b.session()))
    });
    out
}

// ---------------------------------------------------------- work groups --

struct Group<'a> {
    workspace: String,
    work: String,
    run: Option<&'a Value>,
    participants: Vec<usize>,
}

fn run_identity(run: &Value) -> Option<(String, String)> {
    let identity = run.get("identity")?;
    Some((
        str_of(identity, "workspace_id")?.to_owned(),
        str_of(identity, "work_id")?.to_owned(),
    ))
}

fn work_groups<'a>(agents: &[Hosted<'a>], runs: &'a [Value]) -> Vec<Group<'a>> {
    let mut assigned = vec![false; agents.len()];
    let mut groups = Vec::new();
    let mut sorted: Vec<&Value> = runs.iter().collect();
    sorted.sort_by_key(|run| run_identity(run).unwrap_or_default());
    for run in sorted {
        let Some(identity) = run_identity(run) else {
            continue;
        };
        let window = non_empty(run, "window_id");
        let alias_panes: HashSet<&str> = run
            .get("aliases")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|aliases| aliases.values())
            .filter_map(|alias| str_of(alias, "pane"))
            .collect();
        let mut participants = Vec::new();
        for (index, hosted) in agents.iter().enumerate() {
            if assigned[index] {
                continue;
            }
            let Some(pane) = hosted.pane else { continue };
            let belongs = match hosted.work_identity() {
                Some(stamped) => stamped == identity,
                None if !hosted.host.local => false,
                None => match window {
                    Some(window) => {
                        str_of(pane, "window_id") == Some(window) && {
                            let sockets: HashSet<Option<&str>> = hosted
                                .host
                                .panes
                                .iter()
                                .filter(|p| str_of(p, "window_id") == Some(window))
                                .map(|p| str_of(p, "socket"))
                                .collect();
                            sockets.len() == 1
                        }
                    }
                    None => str_of(pane, "pane_id").is_some_and(|id| alias_panes.contains(id)),
                },
            };
            if belongs {
                assigned[index] = true;
                participants.push(index);
            }
        }
        participants.sort_by(|a, b| participant_order(&agents[*a], &agents[*b]));
        groups.push(Group {
            workspace: identity.0,
            work: identity.1,
            run: Some(run),
            participants,
        });
    }
    // Agents in a pane stamped with a Work no run lists: one observed
    // group per identity.
    let mut observed: Vec<((String, String), Vec<usize>)> = Vec::new();
    for (index, hosted) in agents.iter().enumerate() {
        if assigned[index] {
            continue;
        }
        if let Some(identity) = hosted.work_identity() {
            match observed.iter_mut().find(|(id, _)| *id == identity) {
                Some((_, list)) => list.push(index),
                None => observed.push((identity, vec![index])),
            }
        }
    }
    for ((workspace, work), mut participants) in observed {
        participants.sort_by(|a, b| participant_order(&agents[*a], &agents[*b]));
        groups.push(Group {
            workspace,
            work,
            run: None,
            participants,
        });
    }
    groups
}

fn participant_order(a: &Hosted, b: &Hosted) -> std::cmp::Ordering {
    a.status
        .priority()
        .cmp(&b.status.priority())
        .then(b.host.local.cmp(&a.host.local))
        .then_with(|| a.host.alias.cmp(&b.host.alias))
        .then(a.id.cmp(&b.id))
}

// ---------------------------------------------------------------- text --

/// A prompt as a card title (`FleetBoard.promptTitle`).
pub fn prompt_title(prompt: &str) -> Option<Text> {
    const MARKER: &str = "reply from ";
    let text = prompt.trim_start();
    if text.starts_with("<task-notification>") {
        if let (Some(start), Some(end)) = (text.find("<summary>"), text.find("</summary>")) {
            if end > start {
                if let Some(line) = first_line(&text[start + "<summary>".len()..end]) {
                    return Some(Text::plain(format!("⟳ {line}")));
                }
                return None;
            }
        }
        return Some(Text::key("background_task"));
    }
    let Some(rest) = text
        .strip_prefix("[tend:")
        .and_then(|r| r.find(']').map(|i| &r[i + 1..]))
    else {
        return first_line(text).map(Text::Plain);
    };
    let rest = rest.trim();
    let line = rest.split('\n').next().unwrap_or(rest);
    // `(?:Completed )?reply from \S+`, case-insensitively, found in the
    // line itself: lowercasing first would move byte offsets.
    let found = line.char_indices().find_map(|(at, _)| {
        line.get(at..at + MARKER.len())
            .filter(|candidate| candidate.eq_ignore_ascii_case(MARKER))
            .map(|_| at + MARKER.len())
    });
    if let Some(start) = found {
        let who = line[start..].split_whitespace().next().unwrap_or("");
        if !who.is_empty() {
            return Some(Text::with("reply_ready", vec![("who", Text::plain(who))]));
        }
    }
    let sentence = line.split('.').next().unwrap_or(line);
    first_line(sentence).map(Text::Plain)
}

const GENERIC_NOTICES: [&str; 5] = [
    "needs your permission",
    "waiting for your input",
    "approval required",
    "needs your attention",
    "waiting on askuserquestion",
];

/// What the agent asks of the person, in one line.
fn question(hosted: &Hosted) -> Text {
    if let Status::Asked(question) = &hosted.status {
        return Text::plain(question.clone());
    }
    if let Some(note) = line_of(hosted.agent, "last_notification") {
        let lower = note.to_lowercase();
        if GENERIC_NOTICES.iter().any(|g| lower.contains(g)) {
            return Text::key(if str_of(hosted.agent, "state") == Some("waiting_choice") {
                "asks_to_choose"
            } else {
                "waits_for_answer"
            });
        }
        return Text::plain(note);
    }
    hosted.status.label(hosted.agent)
}

/// What the agent is doing or did last, in one line.
fn status_line(hosted: &Hosted) -> Text {
    line_of(hosted.agent, "recap")
        .or_else(|| line_of(hosted.agent, "last_response"))
        .map_or_else(|| hosted.status.label(hosted.agent), Text::Plain)
}

// ---------------------------------------------------------------- units --

#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    pub name: String,
    pub owner: Option<String>,
    pub state: StageState,
    pub kind: Option<String>,
}

#[allow(clippy::struct_excessive_bools)] // the Mac's FleetWork flags, one each
#[derive(Debug, Clone)]
struct Unit {
    id: String,
    source: Value,
    title: Text,
    repo: Option<String>,
    lane: Lane,
    stages: Vec<Stage>,
    headline: Text,
    since: Option<OffsetDateTime>,
    agents: Vec<usize>,
    helpers: Vec<usize>,
    focus: Option<usize>,
    handled_limit: bool,
    revision: String,
    last_change: Option<OffsetDateTime>,
    orphan_helper: bool,
    probable_helper: bool,
    empty_session: bool,
}

fn revision(agents: &[&Hosted], run: Option<&Value>) -> String {
    let mut parts: Vec<String> = agents
        .iter()
        .map(|h| {
            let at = str_of(h.agent, "state_entered_at")
                .or_else(|| str_of(h.agent, "last_activity_at"))
                .unwrap_or("");
            format!("{}|{}|{}", h.id, str_of(h.agent, "state").unwrap_or(""), at)
        })
        .collect();
    parts.sort();
    if let Some(run) = run {
        parts.push(format!(
            "gen:{}",
            run.get("generation").and_then(Value::as_u64).unwrap_or(0)
        ));
        let mut aliases: Vec<String> = run
            .get("aliases")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(key, alias)| format!("{key}={}", str_of(alias, "status").unwrap_or("")))
            .collect();
        aliases.sort();
        parts.extend(aliases);
    }
    parts.join(";")
}

fn latest_change(agents: &[&Hosted]) -> Option<OffsetDateTime> {
    agents.iter().filter_map(|h| h.time()).max()
}

fn ad_hoc_stage(hosted: &Hosted) -> Stage {
    let name = hosted
        .pane
        .and_then(|p| non_empty(p, "agent_role").or_else(|| non_empty(p, "agent_alias")))
        .map_or_else(|| kind_name(hosted.kind()), str::to_owned);
    Stage {
        name,
        owner: Some(hosted.owner()),
        state: stage_state(None, Some(&hosted.status), str_of(hosted.agent, "state")),
        kind: Some(hosted.kind().to_owned()),
    }
}

fn desired_alias_of(group: &Group, hosted: &Hosted) -> Option<String> {
    let run = group.run?;
    let desired = run.get("desired")?.as_array()?;
    if let Some(alias) = hosted.pane.and_then(|p| str_of(p, "agent_alias")) {
        if desired.iter().any(|d| str_of(d, "alias") == Some(alias)) {
            return Some(alias.to_owned());
        }
    }
    let pane_id = hosted.pane.and_then(|p| str_of(p, "pane_id"))?;
    run.get("aliases")?
        .as_object()?
        .iter()
        .find(|(_, state)| str_of(state, "pane") == Some(pane_id))
        .map(|(key, _)| key.clone())
        // Only an alias the pipeline asks for (WorkModels.desiredAgent).
        .filter(|alias| {
            desired
                .iter()
                .any(|d| str_of(d, "alias") == Some(alias.as_str()))
        })
}

#[allow(clippy::too_many_lines)] // FleetBoard.work(for: group), rule for rule in its order
fn managed_unit(group: &Group, agents: &[Hosted]) -> Unit {
    let participants: Vec<&Hosted> = group.participants.iter().map(|i| &agents[*i]).collect();
    let run_aliases = group
        .run
        .and_then(|r| r.get("aliases"))
        .and_then(Value::as_object);
    let alias_status = |alias: &str| {
        run_aliases
            .and_then(|a| a.get(alias))
            .and_then(|s| str_of(s, "status"))
    };
    let desired_of: Vec<Option<String>> = participants
        .iter()
        .map(|p| desired_alias_of(group, p))
        .collect();
    let mut stages = Vec::new();
    let desired = group
        .run
        .and_then(|r| r.get("desired"))
        .and_then(Value::as_array)
        .filter(|d| !d.is_empty());
    if let Some(desired) = desired {
        for entry in desired {
            let alias = str_of(entry, "alias").unwrap_or("");
            let participant = participants
                .iter()
                .zip(&desired_of)
                .find(|(_, d)| d.as_deref() == Some(alias))
                .map(|(p, _)| *p);
            let name = non_empty(entry, "role").unwrap_or(alias).to_owned();
            stages.push(Stage {
                name,
                owner: participant
                    .map(Hosted::owner)
                    .or_else(|| str_of(entry, "program").map(str::to_owned)),
                state: stage_state(
                    alias_status(alias),
                    participant.map(|p| &p.status),
                    participant.and_then(|p| str_of(p.agent, "state")),
                ),
                kind: participant
                    .map(|p| p.kind().to_owned())
                    .or_else(|| str_of(entry, "program").map(str::to_owned)),
            });
        }
        for (participant, desired) in participants.iter().zip(&desired_of) {
            if desired.is_none() {
                stages.push(ad_hoc_stage(participant));
            }
        }
    } else {
        stages.extend(participants.iter().map(|p| ad_hoc_stage(p)));
    }
    let lane = lane_for(&stages);
    let attention = participants
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status.needs_attention())
        .min_by_key(|(_, p)| p.status.priority())
        .map(|(i, _)| i);
    let active = participants
        .iter()
        .position(|p| p.status == Status::Working);
    let handled: HashSet<&str> = participants
        .iter()
        .zip(&desired_of)
        .filter(|(p, _)| p.status == Status::Limited { covered: true })
        .filter_map(|(_, d)| d.as_deref())
        .collect();
    let blocked = run_aliases.and_then(|aliases| {
        let mut keys: Vec<&String> = aliases.keys().collect();
        keys.sort();
        keys.into_iter()
            .find(|key| {
                matches!(alias_status(key), Some("blocked" | "failed"))
                    && !handled.contains(key.as_str())
            })
            .map(|key| {
                (
                    key.clone(),
                    aliases.get(key).cloned().unwrap_or(Value::Null),
                )
            })
    });
    // attention ?? participant(for: blocked) ?? active ?? first
    let blocked_participant = blocked.as_ref().and_then(|(key, _)| {
        desired_of
            .iter()
            .position(|d| d.as_deref() == Some(key.as_str()))
    });
    // attention ?? (blocked ? its participant : active ?? first): a blocked
    // stage with nobody on it has no focus, never another stage's agent.
    let focus = attention.or(if blocked.is_some() {
        blocked_participant
    } else {
        active.or((!participants.is_empty()).then_some(0))
    });
    let headline = if let Some(i) = attention {
        question(participants[i])
    } else if let Some((key, state)) = &blocked {
        str_of(state, "error").and_then(first_line).map_or_else(
            || {
                Text::with(
                    "stage_is",
                    vec![
                        ("stage", Text::plain(key.clone())),
                        ("status", Text::plain(str_of(state, "status").unwrap_or(""))),
                    ],
                )
            },
            Text::Plain,
        )
    } else if let Some(i) = focus {
        status_line(participants[i])
    } else {
        Text::key("waiting_for_agents")
    };
    let cwd = group
        .run
        .and_then(|r| non_empty(r, "cwd"))
        .map(str::to_owned)
        .or_else(|| {
            participants
                .iter()
                .find_map(|p| p.pane.and_then(|pane| non_empty(pane, "current_path")))
                .map(str::to_owned)
        })
        .or_else(|| {
            participants
                .iter()
                .find_map(|p| non_empty(p.agent, "cwd"))
                .map(str::to_owned)
        });
    Unit {
        id: format!("work:{}/{}", group.workspace, group.work),
        source: json!({"kind": "work", "workspace": group.workspace, "work": group.work}),
        title: Text::plain(group.work.clone()),
        repo: cwd.as_deref().and_then(repo_name),
        lane,
        stages,
        headline,
        since: focus.and_then(|i| participants[i].time()),
        agents: group.participants.clone(),
        helpers: Vec::new(),
        focus: focus.map(|i| group.participants[i]),
        handled_limit: participants
            .iter()
            .any(|p| p.status == Status::Limited { covered: true }),
        revision: revision(&participants, group.run),
        last_change: latest_change(&participants),
        orphan_helper: false,
        probable_helper: false,
        empty_session: false,
    }
}

fn solo_unit(index: usize, agents: &[Hosted]) -> Unit {
    let hosted = &agents[index];
    let stage = Stage {
        name: kind_name(hosted.kind()),
        owner: Some(hosted.owner()),
        state: stage_state(None, Some(&hosted.status), str_of(hosted.agent, "state")),
        kind: Some(hosted.kind().to_owned()),
    };
    let lane = lane_for(std::slice::from_ref(&stage));
    let title = non_empty(hosted.agent, "ai_title")
        .map(Text::plain)
        .or_else(|| str_of(hosted.agent, "last_prompt").and_then(prompt_title))
        .unwrap_or_else(|| {
            Text::with(
                "kind_on_host",
                vec![
                    ("kind", Text::plain(kind_name(hosted.kind()))),
                    ("host", Text::plain(hosted.host.alias.clone())),
                ],
            )
        });
    let repo = hosted
        .pane
        .and_then(|p| non_empty(p, "current_path"))
        .and_then(repo_name)
        .or_else(|| non_empty(hosted.agent, "cwd").and_then(repo_name));
    Unit {
        id: format!("agent:{}:{}", hosted.host.alias, hosted.session()),
        source: json!({"kind": "agent", "host": hosted.host.alias, "session_id": hosted.session()}),
        title,
        repo,
        lane,
        stages: vec![stage],
        headline: if hosted.status.needs_attention() {
            question(hosted)
        } else {
            status_line(hosted)
        },
        since: hosted.time(),
        agents: vec![index],
        helpers: Vec::new(),
        focus: Some(index),
        handled_limit: hosted.status == Status::Limited { covered: true },
        revision: revision(&[hosted], None),
        last_change: hosted.time(),
        orphan_helper: false,
        probable_helper: false,
        empty_session: false,
    }
}

// -------------------------------------------------------------- helpers --

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperRequest {
    pub session_id: Option<String>,
    pub label: Option<String>,
    pub confirmed: bool,
}

/// Whether a prompt is another agent's request this session took
/// (`FleetBoard.helperRequest`).
pub fn helper_request(prompt: &str) -> Option<HelperRequest> {
    let text = prompt.trim_start();
    if text.starts_with("[tend] Claimed;") {
        let start = text.find("--- peer request ---\n")? + "--- peer request ---\n".len();
        let end = text[start..].find("\n--- end peer request ---")? + start;
        let record: Value = serde_json::from_str(&text[start..end]).ok()?;
        let from = record.get("from")?;
        let session_id = str_of(from, "agent_session_id").map(str::to_owned);
        let label = str_of(from, "label").map(str::to_owned);
        if session_id.is_none() && label.is_none() {
            return None;
        }
        return Some(HelperRequest {
            session_id,
            label,
            confirmed: true,
        });
    }
    let line = text.lines().next().unwrap_or("");
    if line.starts_with("[tend:req_") {
        if line.contains("Completed reply") || line.contains("replies are ready") {
            return None;
        }
        for marker in ["representing ", " from "] {
            if let Some(at) = line.find(marker) {
                let token = line[at + marker.len()..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                let token = token.trim_matches(|c| ",.;()".contains(c));
                if token.contains("@%") {
                    return Some(HelperRequest {
                        session_id: None,
                        label: Some(token.to_owned()),
                        confirmed: true,
                    });
                }
            }
        }
        return None;
    }
    if line.starts_with('[') {
        return None;
    }
    // A request handed over by typing its id into the agent.
    let handed = regex_lite_handover(line)?;
    Some(HelperRequest {
        session_id: None,
        label: Some(format!("agent@%{handed}")),
        confirmed: false,
    })
}

/// `(tend msg accept|tend_inbox(request_id="?)\s*req_<hex8+>_<hex+>` plus a
/// pane token `%N` not glued to letters or hex: the pane number.
fn regex_lite_handover(line: &str) -> Option<String> {
    let accept = [
        "tend msg accept",
        "tend_inbox(request_id=\"",
        "tend_inbox(request_id=",
    ]
    .iter()
    .find_map(|marker| line.find(marker).map(|at| &line[at + marker.len()..]))?;
    let id = accept.trim_start().strip_prefix("req_")?;
    let (first, rest) = id.split_once('_')?;
    let second: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    if first.len() < 8 || !first.chars().all(|c| c.is_ascii_hexdigit()) || second.is_empty() {
        return None;
    }
    let bytes: Vec<char> = line.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c != '%' {
            continue;
        }
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let digits: String = bytes[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            continue;
        }
        let next = bytes.get(i + 1 + digits.len());
        if next.is_some_and(char::is_ascii_hexdigit) {
            continue;
        }
        return Some(digits);
    }
    None
}

/// `host|socket|pane` from a pane the scan resolved (or `pane` on that
/// pane's server), never from the agent's own unverified pane fields.
fn resolved_pane_key(h: &Hosted, pane: Option<&str>) -> Option<String> {
    let resolved = h.pane?;
    let socket = str_of(resolved, "socket").unwrap_or("");
    let pane = pane.or_else(|| str_of(resolved, "pane_id"))?;
    Some(format!("{}|{socket}|{pane}", h.host.alias))
}

struct HelperIndex {
    parent: HashMap<usize, usize>,
    helpers: HashSet<usize>,
}

impl HelperIndex {
    fn new(agents: &[Hosted]) -> Self {
        let mut by_session: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut by_pane: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, h) in agents.iter().enumerate() {
            by_session.entry(h.session()).or_default().push(i);
            if let Some(key) = resolved_pane_key(h, None) {
                by_pane.entry(key).or_default().push(i);
            }
        }
        let mut index = HelperIndex {
            parent: HashMap::new(),
            helpers: HashSet::new(),
        };
        for (i, h) in agents.iter().enumerate() {
            let Some(request) = str_of(h.agent, "last_prompt").and_then(helper_request) else {
                continue;
            };
            if !request.confirmed {
                continue;
            }
            index.helpers.insert(i);
            let candidates: Vec<usize> = if let Some(session) = &request.session_id {
                by_session
                    .get(session.as_str())
                    .cloned()
                    .unwrap_or_default()
            } else if let (Some(label), Some(key)) = (
                &request.label,
                // A label names a pane on the helper's own server: only a
                // helper whose pane the scan resolved can be placed.
                request
                    .label
                    .as_deref()
                    .and_then(|label| label.rsplit_once('@'))
                    .and_then(|(_, pane)| resolved_pane_key(h, Some(pane))),
            ) {
                let _ = label;
                by_pane.get(&key).cloned().unwrap_or_default()
            } else {
                Vec::new()
            };
            let others: Vec<usize> = candidates.into_iter().filter(|c| *c != i).collect();
            if others.len() == 1 {
                index.parent.insert(i, others[0]);
            }
        }
        index
    }

    /// The top of a helper chain: the first ancestor with no parent; none
    /// for a loop or an agent that helps no one found.
    /// Every agent's root: the first ancestor with no parent, or none for
    /// an agent that helps no one found or whose chain loops. Each chain is
    /// walked once (`FleetBoard` memoizes the same way).
    fn roots(&self, count: usize) -> Vec<Option<usize>> {
        let mut done = vec![false; count];
        let mut on_path = vec![false; count];
        let mut out = vec![None; count];
        for start in 0..count {
            if done[start] {
                continue;
            }
            let mut path = Vec::new();
            let mut current = start;
            let top = loop {
                if done[current] {
                    break if self.parent.contains_key(&current) {
                        out[current]
                    } else {
                        Some(current)
                    };
                }
                if on_path[current] {
                    break None;
                }
                on_path[current] = true;
                path.push(current);
                match self.parent.get(&current) {
                    None => break Some(current),
                    Some(next) => current = *next,
                }
            };
            for node in path {
                out[node] = if self.parent.contains_key(&node) {
                    top
                } else {
                    None
                };
                done[node] = true;
            }
        }
        out
    }
}

fn attach(unit: &mut Unit, helpers: &[usize], agents: &[Hosted]) {
    if helpers.is_empty() {
        return;
    }
    unit.helpers = helpers.to_vec();
    let refs: Vec<&Hosted> = helpers.iter().map(|i| &agents[*i]).collect();
    unit.revision = format!("{};{}", unit.revision, revision(&refs, None));
    unit.last_change = unit.last_change.max(latest_change(&refs));
    unit.handled_limit |= refs
        .iter()
        .any(|h| h.status == Status::Limited { covered: true });
    if unit.lane != Lane::NeedsYou {
        let asking = helpers
            .iter()
            .filter(|i| agents[**i].status.needs_attention())
            .min_by_key(|i| agents[**i].status.priority());
        if let Some(i) = asking {
            let h = &agents[*i];
            unit.lane = Lane::NeedsYou;
            unit.focus = Some(*i);
            unit.headline = Text::with(
                "owner_says",
                vec![("owner", Text::plain(h.owner())), ("text", question(h))],
            );
            unit.since = h.time();
            return;
        }
    }
    if unit.lane == Lane::Finished {
        let running = helpers.iter().find(|i| {
            let h = &agents[**i];
            stage_state(None, Some(&h.status), str_of(h.agent, "state")) == StageState::Running
        });
        if let Some(i) = running {
            let h = &agents[*i];
            unit.lane = Lane::Running;
            unit.headline = Text::with(
                "waiting_on",
                vec![("owner", Text::plain(h.owner())), ("text", status_line(h))],
            );
            unit.since = h.time();
        }
    }
}

fn is_empty_agent(agent: &Value) -> bool {
    ["last_prompt", "last_response", "recap"]
        .iter()
        .all(|key| non_empty(agent, key).is_none())
}

// --------------------------------------------------------------- review --

/// Home's reviewed mark for a unit: the same key the Mac records
/// (`FleetReview.reviewKey`), so a mark made in either is seen in both.
pub fn review_key(id: &str, revision: &str) -> String {
    let digest = Sha256::digest(format!("{id}\n{revision}").as_bytes());
    let mut key = String::with_capacity(5 + 64);
    key.push_str("home:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// Whether a person asked for this unit's work (`isRequestedByPerson`):
/// only those count toward Review.
fn requested_by_person(unit: &Unit, agents: &[Hosted]) -> bool {
    if unit.probable_helper || unit.orphan_helper || unit.empty_session {
        return false;
    }
    let focus = unit.focus.map(|i| &agents[i]);
    match focus.and_then(|h| str_of(h.agent, "last_prompt_origin")) {
        Some("person_via_tend") => return true,
        Some("peer" | "automation" | "tend") => return false,
        _ => {}
    }
    if unit.source.get("kind").and_then(Value::as_str) == Some("work") {
        return true;
    }
    let prompt = focus
        .and_then(|h| non_empty(h.agent, "last_prompt"))
        .unwrap_or("");
    !prompt.is_empty()
        && ![
            "[tend",
            "<task-notification>",
            "[Cross-session",
            "[Request interrupted",
        ]
        .iter()
        .any(|p| prompt.starts_with(p))
}

// ------------------------------------------------------------ decisions --

const TEND_SERVICES: [&str; 4] = [
    "tend:peer-recovery",
    "muxa:peer-recovery",
    "tend:reply-wake",
    "muxa:reply-wake",
];

struct Decision {
    id: String,
    host: String,
    local: bool,
    system: bool,
    since: Option<OffsetDateTime>,
    repo: Option<String>,
    entry: Value,
    actionable: bool,
    /// The asking agent as the board lists it (its pane and socket), when
    /// it is listed.
    asker: Option<Value>,
}

fn decisions(input: &BoardInput, agents: &[Hosted]) -> Vec<Decision> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for section in &input.decisions {
        let host = str_of(section, "alias").unwrap_or("").to_owned();
        let local = section
            .get("local")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let control = section
            .get("control")
            .and_then(Value::as_bool)
            .unwrap_or(local);
        let decides = section
            .get("decides_reviews")
            .and_then(Value::as_bool)
            .unwrap_or(local);
        let items = section
            .pointer("/queue/items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            let (id, since, system, actionable) = match str_of(&item, "kind") {
                Some("human_request") => (
                    format!("human:{host}:{}", str_of(&item, "request_id").unwrap_or("")),
                    parse_time(str_of(&item, "created_at")),
                    item.get("system").and_then(Value::as_bool) == Some(true)
                        || str_of(&item, "from_session_id")
                            .is_some_and(|s| TEND_SERVICES.contains(&s)),
                    local || control,
                ),
                Some("review") => (
                    format!("review:{}", str_of(&item, "id").unwrap_or("")),
                    parse_time(str_of(&item, "requested_at")),
                    false,
                    local || (control && decides),
                ),
                // Waiting agents are units already.
                _ => continue,
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            // The asker's checkout names the project (attachAskers).
            // The asker: who filed the question, or the session a result
            // is about (FleetDecision.askerSessionID).
            let asker = match str_of(&item, "kind") {
                Some("review") => item
                    .get("target")
                    .filter(|t| str_of(t, "kind") == Some("session"))
                    .and_then(|t| str_of(t, "session_id")),
                _ => str_of(&item, "from_session_id"),
            };
            let asking = asker.and_then(|session| {
                agents
                    .iter()
                    .find(|h| h.host.alias == host && h.session() == session)
            });
            let repo = asking
                .and_then(|h| non_empty(h.agent, "cwd"))
                .and_then(repo_name);
            let asker = asking.map(Hosted::reference);
            out.push(Decision {
                id,
                host: host.clone(),
                local,
                system,
                since,
                repo,
                entry: item,
                actionable,
                asker,
            });
        }
    }
    // Agents' questions and results first, tend's own after.
    out.sort_by_key(|d| d.system);
    out
}

// ---------------------------------------------------------------- board --

fn unit_json(
    unit: &Unit,
    agents: &[Hosted],
    reviewed: Option<&HashSet<String>>,
    now: OffsetDateTime,
) -> Value {
    let shown = if unit.lane == Lane::NeedsYou {
        unit.since
    } else {
        unit.last_change.or(unit.since)
    };
    let key = review_key(&unit.id, &unit.revision);
    let mut host_aliases: Vec<String> = Vec::new();
    let mut kinds: Vec<String> = Vec::new();
    let mut programs = HashSet::new();
    for i in unit.agents.iter().chain(&unit.helpers) {
        let h = &agents[*i];
        if !host_aliases.contains(&h.host.alias) {
            host_aliases.push(h.host.alias.clone());
        }
    }
    host_aliases.sort();
    // The marks of the stages, planned ones included, in stage order
    // (FleetWork.agentKinds): a helper does not add a mark.
    for kind in unit.stages.iter().filter_map(|stage| stage.kind.as_deref()) {
        if programs.insert(program_of(kind)) {
            kinds.push(kind.to_owned());
        }
    }
    json!({
        "type": "unit",
        "id": unit.id,
        "source": unit.source,
        "title": unit.title,
        "repo": unit.repo,
        "lane": unit.lane,
        "stages": unit.stages,
        "headline": unit.headline,
        "since": format_time(unit.since),
        "last_change": format_time(unit.last_change),
        "shown_time": format_time(shown),
        "agents": unit.agents.iter().map(|i| agents[*i].reference()).collect::<Vec<_>>(),
        "helpers": unit.helpers.iter().map(|i| agents[*i].reference()).collect::<Vec<_>>(),
        "focus": unit.focus.map(|i| agents[i].reference()),
        "host_aliases": host_aliases,
        "agent_kinds": kinds,
        "handled_limit": unit.handled_limit,
        "orphan_helper": unit.orphan_helper,
        "probable_helper": unit.probable_helper,
        "empty_session": unit.empty_session,
        "recent": unit.lane != Lane::Finished || shown.is_none_or(|t| now - t <= RECENT_FINISH),
        "revision": unit.revision,
        "review_key": key,
        "reviewed": unit.lane == Lane::Finished && reviewed.is_some_and(|set| set.contains(&key)),
        "requested_by_person": requested_by_person(unit, agents),
    })
}

fn decision_json(decision: &Decision) -> Value {
    json!({
        "type": "decision",
        "id": decision.id,
        "host": decision.host,
        "local": decision.local,
        "system": decision.system,
        "repo": decision.repo,
        "since": format_time(decision.since),
        "actionable": decision.actionable,
        "asker": decision.asker,
        "entry": decision.entry,
    })
}

/// Items of one lane grouped by project, a project where its first item
/// is, tend's own asks last in their own group.
fn grouped(items: Vec<(Option<String>, bool, Value)>) -> Vec<Value> {
    let mut order: Vec<Option<String>> = Vec::new();
    let mut groups: HashMap<Option<String>, Vec<Value>> = HashMap::new();
    let mut system = Vec::new();
    for (repo, is_system, item) in items {
        if is_system {
            system.push(item);
            continue;
        }
        if !groups.contains_key(&repo) {
            order.push(repo.clone());
        }
        groups.entry(repo).or_default().push(item);
    }
    let mut out: Vec<Value> = order
        .into_iter()
        .map(|repo| json!({"project": repo, "system": false, "items": groups.remove(&repo).unwrap_or_default()}))
        .collect();
    if !system.is_empty() {
        out.push(json!({"project": null, "system": true, "items": system}));
    }
    out
}

fn backlog_json(input: &BoardInput, units: &[Unit]) -> Vec<Value> {
    let running: HashSet<(String, String)> = units
        .iter()
        .filter(|&u| u.source.get("kind").and_then(Value::as_str) == Some("work"))
        .map(|u| {
            (
                str_of(&u.source, "workspace").unwrap_or("").to_owned(),
                str_of(&u.source, "work").unwrap_or("").to_owned(),
            )
        })
        .collect();
    input
        .backlog
        .iter()
        .filter(|record| {
            record.pointer("/metadata/stage").and_then(Value::as_str) == Some("queued")
        })
        .filter(|record| {
            let identity = record.get("identity").cloned().unwrap_or(Value::Null);
            let id = (
                str_of(&identity, "workspace_id").unwrap_or("").to_owned(),
                str_of(&identity, "work_id").unwrap_or("").to_owned(),
            );
            !running.contains(&id)
        })
        .map(|record| {
            let identity = record.get("identity").cloned().unwrap_or(Value::Null);
            let metadata = record.get("metadata").cloned().unwrap_or(Value::Null);
            let plan = record.get("plan").cloned().unwrap_or(Value::Null);
            let work = str_of(&identity, "work_id").unwrap_or("");
            let line = ["goal", "next_action"]
                .iter()
                .find_map(|k| line_of(&metadata, k))
                .or_else(|| line_of(&plan, "prompt"));
            json!({
                "workspace": str_of(&identity, "workspace_id"),
                "work": work,
                "title": non_empty(&metadata, "title").unwrap_or(work),
                "line": line,
                "priority": plan.get("priority"),
                "pipeline": str_of(&plan, "pipeline"),
                "external": str_of(&plan, "external"),
                "project": non_empty(&plan, "cwd").and_then(repo_name),
            })
        })
        .collect()
}

/// One Your turn entry: system ask, wait start, original position,
/// project, the item.
type TurnEntry = (bool, Option<OffsetDateTime>, usize, Option<String>, Value);

/// The whole board: lanes of units and decisions in the Mac's order,
/// grouped by project, with each unit's fold flags and reviewed mark.
#[allow(clippy::too_many_lines)] // FleetBoard.works + yourTurn + lanes, in that order
pub fn build(input: &BoardInput) -> Value {
    let now = input.now.unwrap_or_else(OffsetDateTime::now_utc);
    let coverage = Coverage::new(input);
    let agents = hosted_agents(input, &coverage, now);
    let groups = work_groups(&agents, &input.runs);
    let managed: HashSet<usize> = groups
        .iter()
        .flat_map(|g| g.participants.iter().copied())
        .collect();
    let index = HelperIndex::new(&agents);
    let roots = index.roots(agents.len());
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut folded = HashSet::new();
    for (i, root) in roots.iter().enumerate() {
        if managed.contains(&i) {
            continue;
        }
        if let Some(root) = *root {
            by_root.entry(root).or_default().push(i);
            folded.insert(i);
        }
    }
    let mut units = Vec::new();
    for group in &groups {
        let mut unit = managed_unit(group, &agents);
        let helpers: Vec<usize> = group
            .participants
            .iter()
            .flat_map(|p| by_root.get(p).cloned().unwrap_or_default())
            .collect();
        attach(&mut unit, &helpers, &agents);
        units.push(unit);
    }
    for i in 0..agents.len() {
        if managed.contains(&i) || folded.contains(&i) {
            continue;
        }
        let mut unit = solo_unit(i, &agents);
        let helpers = by_root.get(&i).cloned().unwrap_or_default();
        attach(&mut unit, &helpers, &agents);
        unit.orphan_helper = index.helpers.contains(&i);
        unit.probable_helper = str_of(agents[i].agent, "last_prompt")
            .and_then(helper_request)
            .is_some_and(|r| !r.confirmed);
        unit.empty_session = unit.helpers.is_empty() && is_empty_agent(agents[i].agent);
        units.push(unit);
    }
    let sort_time = |u: &Unit| {
        if u.lane == Lane::NeedsYou {
            u.since
        } else {
            u.last_change.or(u.since)
        }
    };
    units.sort_by(|a, b| {
        a.lane.cmp(&b.lane).then_with(|| {
            let (ta, tb) = (sort_time(a), sort_time(b));
            let by_time = if a.lane == Lane::NeedsYou {
                ta.cmp(&tb)
            } else {
                tb.cmp(&ta)
            };
            by_time.then(a.id.cmp(&b.id))
        })
    });
    let reviewed = input.reviewed.as_ref();
    let decisions = decisions(input, &agents);
    // Your turn: decisions and units, the longest wait first; tend's own
    // asks after everything an agent asked; a decision first on a tie.
    let mut turn: Vec<TurnEntry> = Vec::new();
    for (position, decision) in decisions.iter().enumerate() {
        turn.push((
            decision.system,
            decision.since,
            position,
            decision.repo.clone(),
            decision_json(decision),
        ));
    }
    let offset = decisions.len();
    for (position, unit) in units
        .iter()
        .filter(|u| u.lane == Lane::NeedsYou)
        .enumerate()
    {
        turn.push((
            false,
            unit.since,
            offset + position,
            unit.repo.clone(),
            unit_json(unit, &agents, reviewed, now),
        ));
    }
    turn.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let needs_count = turn.len();
    let needs_you = grouped(
        turn.into_iter()
            .map(|(system, _, _, repo, item)| (repo, system, item))
            .collect(),
    );
    let lane_of = |lane: Lane| {
        let items: Vec<(Option<String>, bool, Value)> = units
            .iter()
            .filter(|u| u.lane == lane)
            .map(|u| (u.repo.clone(), false, unit_json(u, &agents, reviewed, now)))
            .collect();
        let count = items.len();
        json!({"count": count, "groups": grouped(items)})
    };
    let mut projects: Vec<String> = units
        .iter()
        .filter_map(|u| u.repo.clone())
        .chain(decisions.iter().filter_map(|d| d.repo.clone()))
        .collect();
    projects.sort();
    projects.dedup();
    json!({
        "generated_at": format_time(Some(now)),
        "hosts": input.hosts.iter().map(|h| json!({
            "alias": h.alias, "local": h.local, "state": h.state, "error": h.error,
        })).collect::<Vec<_>>(),
        "lanes": {
            "needs_you": {"count": needs_count, "groups": needs_you},
            "running": lane_of(Lane::Running),
            "finished": lane_of(Lane::Finished),
        },
        "backlog": backlog_json(input, &units),
        "projects": projects,
        "reviewed_marks": input.reviewed.is_some(),
        // What each host's queue covers and could not read, so a client can
        // tell an empty Your turn from one it could not see.
        "decision_hosts": input.decisions.iter().map(|section| json!({
            "alias": section.get("alias"),
            "local": section.get("local"),
            "control": section.get("control"),
            "decides_reviews": section.get("decides_reviews"),
            "covers": section.get("covers"),
            "truncated": section.pointer("/queue/truncated"),
            "unavailable": section.get("unavailable"),
        })).collect::<Vec<_>>(),
        "agent_count": agents.len(),
    })
}

/// The review keys of the board's finished units, to look their marks up.
#[must_use]
pub fn finished_keys(board: &Value) -> Vec<String> {
    board
        .pointer("/lanes/finished/groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|item| {
            item.get("review_key")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

/// The `home:` keys of Home's accepted review records, read from review
/// items as `review_list` returns them.
pub fn reviewed_keys(items: &[Value]) -> HashSet<String> {
    items
        .iter()
        .filter(|item| {
            item.pointer("/disposition/kind").and_then(Value::as_str) == Some("accepted")
        })
        .filter_map(|item| str_of(item, "idempotency_key"))
        .filter(|key| key.starts_with("home:"))
        .map(str::to_owned)
        .collect()
}

/// A JSON object helper for callers that assemble hosts from Fleet.
pub fn host_from_snapshot(snapshot: &Value) -> BoardHost {
    let remote = snapshot.get("remote").cloned().unwrap_or(Value::Null);
    let list = |key: &str| {
        remote
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    BoardHost {
        alias: str_of(snapshot, "alias").unwrap_or("").to_owned(),
        local: snapshot
            .get("local")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        state: str_of(snapshot, "state").unwrap_or("unknown").to_owned(),
        error: str_of(snapshot, "error").map(str::to_owned),
        agents: list("agents"),
        panes: list("panes"),
    }
}

mod sources;
pub use sources::{accepted, HomeSources, BOARD_REUSE};

#[cfg(test)]
mod tests;
