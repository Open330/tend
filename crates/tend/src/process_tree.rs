//! Best-effort process-tree enrichment for pane-backed agents.
//!
//! tmux tells us the pane's initial process (`pane_pid`) and foreground
//! command, but not the tree below the agent. Agent CLIs can spawn shells,
//! helper MCP servers, or nested agent sessions; this module walks the OS
//! process tree under a pane and reduces it to a small, argv-free summary
//! suitable for storing in the live registry.

use crate::discovery::classify_command;
use crate::event::AgentKind;
use crate::process_snapshot::ProcessInfo;
use crate::tmux::PaneInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::collections::{HashSet, VecDeque};

const MAX_DEPTH: u8 = 10;
const MAX_NODES: usize = 256;
const MAX_PREVIEW: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadProcessKind {
    Shell,
    Subagent,
    Helper,
    Process,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadProcess {
    pub pid: u32,
    pub parent_pid: u32,
    pub depth: u8,
    pub kind: WorkloadProcessKind,
    pub command: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_pid: Option<u32>,
    #[serde(default)]
    pub process_count: u16,
    #[serde(default)]
    pub shell_count: u16,
    #[serde(default)]
    pub subagent_count: u16,
    #[serde(default)]
    pub helper_count: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preview: Vec<WorkloadProcess>,
}

impl WorkloadSummary {
    /// Nothing at all runs in the pane: no agent, no shell, no child. An agent
    /// with no children of its own is not empty: dropping it here once made
    /// the per-pane scan lose 40 of 50 live agents.
    pub fn is_empty(&self) -> bool {
        self.primary_pid.is_none()
            && self.process_count == 0
            && self.shell_count == 0
            && self.subagent_count == 0
            && self.helper_count == 0
            && self.preview.is_empty()
    }

    pub fn visible_count(&self) -> u16 {
        self.process_count
    }
}

/// Scan every pane and return its workload summary keyed by pane id.
///
/// Every scanned pane has an entry, empty or not, so a missing key means "not
/// scanned this pass" and never "nothing runs there". Callers that only want
/// to clear stale data can still treat a missing key as
/// [`WorkloadSummary::default`].
#[cfg(target_os = "linux")]
pub fn scan_pane_workloads(panes: &[PaneInfo]) -> HashMap<String, WorkloadSummary> {
    let mut out = HashMap::new();
    // Each pane once, not once per `~view~` listing of a grouped session.
    for pane in &crate::tmux::canonical_panes(panes) {
        if pane.pane_pid == 0 {
            continue;
        }
        out.insert(pane.pane_id.clone(), scan_pane_workload(pane));
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn scan_pane_workloads(panes: &[PaneInfo]) -> HashMap<String, WorkloadSummary> {
    if !panes.iter().any(|pane| pane.pane_pid != 0) {
        return HashMap::new();
    }
    scan_pane_workloads_with(panes, &ProcessSource::capture())
}

/// One process-table read shared by several scans of the same moment. The
/// reconciler takes one per tick for both its workload and Codex scans,
/// instead of one `ps` pass each. On Linux it is empty: `/proc` is read
/// per pane, which is already cheap.
#[derive(Debug, Default)]
pub struct ProcessSource {
    #[cfg(not(target_os = "linux"))]
    table: crate::process_snapshot::ProcessTable,
}

impl ProcessSource {
    /// Read the process table now (a no-op on Linux).
    #[must_use]
    pub fn capture() -> Self {
        Self {
            #[cfg(not(target_os = "linux"))]
            table: crate::process_snapshot::read_current_process_table(),
        }
    }
}

/// [`scan_pane_workloads`] over an already captured [`ProcessSource`].
#[cfg(target_os = "linux")]
pub fn scan_pane_workloads_with(
    panes: &[PaneInfo],
    _source: &ProcessSource,
) -> HashMap<String, WorkloadSummary> {
    scan_pane_workloads(panes)
}

/// [`scan_pane_workloads`] over an already captured [`ProcessSource`].
#[cfg(not(target_os = "linux"))]
pub fn scan_pane_workloads_with(
    panes: &[PaneInfo],
    source: &ProcessSource,
) -> HashMap<String, WorkloadSummary> {
    let table = &source.table;
    let mut out = HashMap::new();
    for pane in &crate::tmux::canonical_panes(panes) {
        if pane.pane_pid == 0 {
            continue;
        }
        let summary = summarize(table.descendants(pane.pane_pid, MAX_DEPTH, MAX_NODES));
        out.insert(pane.pane_id.clone(), summary);
    }
    out
}

/// A running agent explicitly started on a known session, and the one pane it
/// runs in.
#[derive(Debug, Clone)]
pub struct ResumeBinding {
    pub session_id: String,
    pub kind: AgentKind,
    pub pane: PaneInfo,
}

/// Live Codex evidence from one walk of every pane's process tree.
#[derive(Debug, Default)]
pub struct CodexPaneScan {
    /// Explicit resume ids (`codex resume <id>`, `claude --resume <id>`) and
    /// the one pane each runs in. A multiplexer restart renumbers panes and
    /// `tend reload` relaunches agents this way, so this is how a registry row
    /// finds its new pane before the agent's next hook.
    pub resume_bindings: Vec<ResumeBinding>,
    /// Panes running an interactive Codex, resumed or fresh. A thread behind
    /// the shared app-server reports no pane, and the screen detector can
    /// claim the pane's placeholder row as `Unknown`, so this — not a registry
    /// placeholder — is the evidence a cwd correlation needs.
    pub hosts: Vec<PaneInfo>,
    /// Per pane (id and endpoint), which agents run in it and which of them
    /// the walk can name. A question's subject check reads it: a session
    /// resumed and not yet prompted has no live row, and its own process is
    /// the only sign it is back (`subject_check.rs`).
    pub identities: HashMap<PaneKey, PaneIdentities>,
    /// The walk covered every pane it was given: the process table was
    /// read, every pane had a root process, and no walk stopped at its depth
    /// or node limit. Only then does what [`Self::identities`] lacks say
    /// anything.
    pub complete: bool,
}

/// Recover detached Codex sessions from an explicit CLI resume id and its
/// pane ancestry, and list the panes hosting an interactive Codex. Never
/// guesses from cwd or chooses between multiple panes itself.
pub fn scan_codex_panes(panes: &[PaneInfo]) -> CodexPaneScan {
    scan_codex_panes_with(panes, &ProcessSource::capture())
}

/// [`scan_codex_panes`] over an already captured [`ProcessSource`].
pub fn scan_codex_panes_with(panes: &[PaneInfo], source: &ProcessSource) -> CodexPaneScan {
    #[cfg(not(target_os = "linux"))]
    let table = &source.table;
    #[cfg(target_os = "linux")]
    let _ = source;
    // A grouped session lists each pane once per `~view~` client session:
    // scan (and report) each pane once, under its base session.
    let unique = crate::tmux::canonical_panes(panes);
    let scanned: Vec<_> = unique
        .iter()
        .filter(|p| p.pane_pid != 0)
        .map(|pane| {
            #[cfg(not(target_os = "linux"))]
            let (processes, whole) = table.descendants_walk(pane.pane_pid, MAX_DEPTH, MAX_NODES);
            #[cfg(target_os = "linux")]
            let (processes, whole) = read_descendants_walk(pane.pane_pid);
            // An unreadable root is no root: the walk is not whole.
            #[cfg(target_os = "linux")]
            let root = read_proc_checked(pane.pane_pid, 0).ok().flatten();
            #[cfg(not(target_os = "linux"))]
            let root = table.get(pane.pane_pid).cloned();
            // A directly exec'd TUI can itself be the pane root process.
            let walked = root.is_some() && whole;
            let processes: Vec<ProcessInfo> = root.into_iter().chain(processes).collect();
            (pane, processes, walked)
        })
        .collect();
    #[cfg(not(target_os = "linux"))]
    let read = !table.is_empty();
    #[cfg(target_os = "linux")]
    let read = true;
    let complete = read
        && unique.iter().all(|pane| pane.pane_pid != 0)
        && scanned.iter().all(|(_, _, walked)| *walked);
    let identities = scanned
        .iter()
        .map(|(pane, processes, _)| (PaneKey::of(pane), pane_identities(processes)))
        .collect();
    let scanned: Vec<_> = scanned
        .into_iter()
        .map(|(pane, processes, _)| (pane, processes))
        .collect();
    let hosts = scanned
        .iter()
        .filter(|(_, processes)| {
            processes.iter().any(is_interactive_codex) && !isolated_codex_host(processes)
        })
        .map(|(pane, _)| (*pane).clone())
        .collect();
    CodexPaneScan {
        identities,
        complete,
        resume_bindings: unique_resume_bindings(
            scanned
                .into_iter()
                .filter(|(_, processes)| !isolated_codex_host(processes)),
        ),
        hosts,
    }
}

// A no-daemon TUI owns its hooks and cannot host a shared-server thread.
// Ignore nested helpers: the outermost interactive TUI owns this pane.
fn isolated_codex_host(processes: &[ProcessInfo]) -> bool {
    processes
        .iter()
        .filter(|process| is_interactive_codex(process))
        .min_by_key(|process| (process.depth, process.pid))
        .is_some_and(|process| {
            process
                .cmdline
                .split_whitespace()
                .any(|arg| arg == "--no-daemon")
        })
}

/// A Codex TUI (`codex`, `codex --yolo`, `codex resume …`) rather than a
/// headless `exec` run or an `app-server`, which serves other panes' threads.
fn is_interactive_codex(process: &ProcessInfo) -> bool {
    if agent_kind(process) != Some(AgentKind::Codex) {
        return false;
    }
    let mut args = process.cmdline.split_whitespace();
    if !args.any(|arg| crate::shell::program_name(arg) == "codex") {
        return false;
    }
    !matches!(
        args.find(|arg| !arg.starts_with('-')),
        Some("app-server" | "exec" | "e" | "review" | "mcp-server")
    )
}

/// One physical pane: its id and normalized endpoint, as
/// `subject_evidence::ScannedPane` carries them. Pane ids repeat across
/// sockets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaneKey {
    pub pane_id: String,
    pub endpoint: Option<String>,
}

impl PaneKey {
    #[must_use]
    pub fn of(pane: &PaneInfo) -> Self {
        Self {
            pane_id: pane.pane_id.clone(),
            endpoint: pane
                .socket
                .as_deref()
                .map(|socket| crate::backend::pane_endpoint_identity(Some(&pane.pane_id), socket)),
        }
    }
}

/// The agents one pane's walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneIdentities {
    /// Any agent process at all.
    pub agent: bool,
    /// Session ids a process in it was explicitly started on, whichever way
    /// its hooks run (`--no-daemon` included).
    pub resumed: Vec<String>,
    /// Top-level agent processes (no agent among their ancestors in the
    /// pane) that carry no session id: `claude --continue`, a fresh
    /// `codex`, a session picked from a list. An agent's own children are
    /// part of it, not counted.
    pub unnamed: u16,
}

/// What [`PaneIdentities`] a pane's walk shows.
fn pane_identities(processes: &[ProcessInfo]) -> PaneIdentities {
    let agents: HashMap<u32, &ProcessInfo> = processes
        .iter()
        .filter(|process| agent_kind(process).is_some())
        .map(|process| (process.pid, process))
        .collect();
    let parent_of: HashMap<u32, u32> = processes.iter().map(|p| (p.pid, p.parent_pid)).collect();
    // The nearest agent among a process's ancestors in the pane.
    let nearest_agent = |process: &ProcessInfo| {
        let mut pid = process.parent_pid;
        for _ in 0..=usize::from(MAX_DEPTH) + 1 {
            if let Some(agent) = agents.get(&pid) {
                return Some(*agent);
            }
            pid = *parent_of.get(&pid)?;
        }
        None
    };
    // An agent process is one more session unless it is provably the same
    // one as the agent it runs under: a launcher shim and the binary it
    // starts carry the same kind and the same session id, or the same
    // arguments. Any other agent below an agent (`codex resume a` -> shell
    // -> `claude --continue`) is its own session.
    let unnamed = agents
        .values()
        .filter(|process| resume_id(process).is_none())
        .filter(|process| {
            nearest_agent(process).is_none_or(|parent| !same_session(parent, process))
        })
        .count();
    let mut resumed: Vec<String> = resumed_sessions(std::iter::once(processes))
        .into_iter()
        .collect();
    resumed.sort();
    PaneIdentities {
        agent: !agents.is_empty(),
        resumed,
        unnamed: u16::try_from(unnamed).unwrap_or(u16::MAX),
    }
}

/// Whether `child`, an agent process under the agent `parent`, is the same
/// session: the same kind, and the same explicit session id, or no id, the
/// same arguments, and `child` started directly by `parent` (a shim
/// re-executing the real binary). Through anything in between (a shell),
/// two agents with the same flags can each pick their own session.
fn same_session(parent: &ProcessInfo, child: &ProcessInfo) -> bool {
    let args = |process: &ProcessInfo| -> Vec<String> {
        process
            .cmdline
            .split_whitespace()
            .skip(1)
            .map(str::to_owned)
            .collect()
    };
    agent_kind(parent) == agent_kind(child)
        && match (resume_id(parent), resume_id(child)) {
            (Some(a), Some(b)) => a == b,
            (None, None) => child.parent_pid == parent.pid && args(parent) == args(child),
            _ => false,
        }
}

/// Every session id a process in these panes was explicitly started on.
fn resumed_sessions<'a>(
    panes: impl Iterator<Item = &'a [ProcessInfo]>,
) -> std::collections::BTreeSet<String> {
    panes
        .flat_map(|processes| processes.iter().filter_map(resume_id))
        .map(str::to_owned)
        .collect()
}

fn unique_resume_bindings<'a>(
    scanned: impl Iterator<Item = (&'a PaneInfo, Vec<ProcessInfo>)>,
) -> Vec<ResumeBinding> {
    let mut candidates: HashMap<String, (AgentKind, Vec<PaneInfo>)> = HashMap::new();
    for (pane, processes) in scanned {
        for process in processes {
            let (Some(sid), Some(kind)) = (resume_id(&process), agent_kind(&process)) else {
                continue;
            };
            let (_, matches) = candidates
                .entry(sid.to_owned())
                .or_insert_with(|| (kind, Vec::new()));
            if !matches
                .iter()
                .any(|p| p.pane_id == pane.pane_id && p.socket == pane.socket)
            {
                matches.push(pane.clone());
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|(session_id, (kind, mut panes))| {
            (panes.len() == 1).then(|| ResumeBinding {
                session_id,
                kind,
                pane: panes.remove(0),
            })
        })
        .collect()
}

/// The session an agent process was explicitly started on, if any.
fn resume_id(process: &ProcessInfo) -> Option<&str> {
    match agent_kind(process) {
        Some(AgentKind::Codex) => codex_resume_id(process),
        Some(AgentKind::ClaudeCode) => claude_resume_id(process),
        _ => None,
    }
}

/// `claude --resume <id>` / `-r <id>` / `--resume=<id>`, and the explicit
/// `--session-id <id>` a forked session is started with.
fn claude_resume_id(process: &ProcessInfo) -> Option<&str> {
    let mut args = process.cmdline.split_whitespace();
    while let Some(arg) = args.next() {
        let sid = match arg {
            "--resume" | "-r" | "--session-id" => args.next()?,
            _ => match arg
                .strip_prefix("--resume=")
                .or_else(|| arg.strip_prefix("--session-id="))
            {
                Some(sid) => sid,
                None => continue,
            },
        };
        return uuid::Uuid::parse_str(sid).ok().map(|_| sid);
    }
    None
}

fn codex_resume_id(process: &ProcessInfo) -> Option<&str> {
    if agent_kind(process) != Some(AgentKind::Codex) {
        return None;
    }
    let mut args = process.cmdline.split_whitespace();
    while let Some(arg) = args.next() {
        if matches!(arg, "exec" | "app-server" | "review" | "fork") {
            return None;
        }
        if arg == "resume" {
            let sid = args.next()?;
            return uuid::Uuid::parse_str(sid).ok().map(|_| sid);
        }
    }
    None
}

pub fn scan_pane_workload(pane: &PaneInfo) -> WorkloadSummary {
    if pane.pane_pid == 0 {
        return WorkloadSummary::default();
    }
    summarize(read_descendants(pane.pane_pid))
}

fn summarize(procs: Vec<ProcessInfo>) -> WorkloadSummary {
    let parent_by_pid: HashMap<u32, u32> = procs.iter().map(|p| (p.pid, p.parent_pid)).collect();
    let primary_pid = procs
        .iter()
        .filter(|p| agent_kind(p).is_some())
        .min_by_key(|p| (p.depth, p.pid))
        .map(|p| p.pid);

    let mut summary = WorkloadSummary {
        primary_pid,
        ..WorkloadSummary::default()
    };

    for proc in &procs {
        if Some(proc.pid) == primary_pid {
            continue;
        }
        if let Some(primary) = primary_pid {
            if !has_ancestor(proc.pid, primary, &parent_by_pid) {
                continue;
            }
        }

        let kind = classify_workload_process(proc);
        let command = display_command(proc, kind);
        if kind == WorkloadProcessKind::Helper {
            summary.helper_count = summary.helper_count.saturating_add(1);
            continue;
        }

        summary.process_count = summary.process_count.saturating_add(1);
        match kind {
            WorkloadProcessKind::Shell => {
                summary.shell_count = summary.shell_count.saturating_add(1);
            }
            WorkloadProcessKind::Subagent => {
                summary.subagent_count = summary.subagent_count.saturating_add(1);
            }
            WorkloadProcessKind::Helper | WorkloadProcessKind::Process => {}
        }
        if summary.preview.len() < MAX_PREVIEW {
            summary.preview.push(WorkloadProcess {
                pid: proc.pid,
                parent_pid: proc.parent_pid,
                depth: proc.depth,
                kind,
                command,
            });
        }
    }
    summary
}

fn has_ancestor(pid: u32, ancestor: u32, parent_by_pid: &HashMap<u32, u32>) -> bool {
    let mut cur = pid;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = parent_by_pid.get(&cur).copied() else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent <= 1 || parent == cur {
            return false;
        }
        cur = parent;
    }
    false
}

fn classify_workload_process(proc: &ProcessInfo) -> WorkloadProcessKind {
    if is_helper(proc) {
        WorkloadProcessKind::Helper
    } else if agent_kind(proc).is_some() {
        WorkloadProcessKind::Subagent
    } else if is_shell(proc) {
        WorkloadProcessKind::Shell
    } else {
        WorkloadProcessKind::Process
    }
}

fn agent_kind(proc: &ProcessInfo) -> Option<AgentKind> {
    classify_command(command_name(&proc.comm)).or_else(|| {
        if is_claude_fork_session(&proc.cmdline) {
            Some(AgentKind::ClaudeCode)
        } else {
            None
        }
    })
}

fn is_claude_fork_session(cmdline: &str) -> bool {
    cmdline.contains("--fork-session")
        && cmdline.contains("--session-id")
        && (cmdline.contains("/claude/versions/") || cmdline.contains(".claude"))
}

fn is_helper(proc: &ProcessInfo) -> bool {
    let comm = command_name(&proc.comm).to_ascii_lowercase();
    let cmdline = proc.cmdline.to_ascii_lowercase();
    comm.ends_with("-mcp")
        || cmdline.contains("@playwright/mcp")
        || cmdline.contains("@modelcontextprotocol/")
        || cmdline.contains("playwright-mcp")
        || cmdline.contains("chrome-devtools-mcp")
        || cmdline.contains(" mcp")
        || cmdline.contains("-mcp")
        || comm.starts_with("pyright-langser")
        || cmdline.contains("pyright-langserver")
        || cmdline.contains("langserver.index.js")
        || cmdline.contains("language-server")
        || cmdline.contains("language_server")
        || cmdline.contains("rust-analyzer")
}

fn is_shell(proc: &ProcessInfo) -> bool {
    matches!(
        command_name(&proc.comm).to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "csh" | "tcsh" | "pwsh"
    )
}

fn display_command(proc: &ProcessInfo, kind: WorkloadProcessKind) -> String {
    if kind == WorkloadProcessKind::Subagent {
        if let Some(agent) = agent_kind(proc) {
            return match agent {
                AgentKind::ClaudeCode => "claude".to_string(),
                AgentKind::Codex => "codex".to_string(),
                AgentKind::Antigravity => "agy".to_string(),
                AgentKind::Opencode => "opencode".to_string(),
                AgentKind::Copilot => "copilot".to_string(),
                AgentKind::Task | AgentKind::Unknown => command_name(&proc.comm).to_string(),
            };
        }
    }
    command_name(&proc.comm).to_string()
}

fn command_name(s: &str) -> &str {
    crate::shell::program_name(s.trim()).trim()
}

#[cfg(target_os = "linux")]
pub(crate) fn read_descendants(root: u32) -> Vec<ProcessInfo> {
    read_descendants_walk(root).0
}

/// [`read_descendants`], and whether the walk reached every descendant:
/// `false` past [`MAX_DEPTH`], at [`MAX_NODES`] with more to visit, or when
/// anything under `/proc` could not be read for a process that still
/// exists. A process (or thread) that exits mid-walk is simply gone.
#[cfg(target_os = "linux")]
fn read_descendants_walk(root: u32) -> (Vec<ProcessInfo>, bool) {
    let mut whole = true;
    let mut out = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let children = |pid: u32, whole: &mut bool| {
        read_children_checked(pid).unwrap_or_else(|()| {
            *whole = false;
            Vec::new()
        })
    };
    let mut q: VecDeque<(u32, u8)> = children(root, &mut whole)
        .into_iter()
        .map(|pid| (pid, 1))
        .collect();
    while let Some((pid, depth)) = q.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        if depth > MAX_DEPTH || out.len() >= MAX_NODES {
            whole = false;
            continue;
        }
        match read_proc_checked(pid, depth) {
            Ok(Some(proc)) => {
                for child in children(pid, &mut whole) {
                    q.push_back((child, depth.saturating_add(1)));
                }
                out.push(proc);
            }
            // Gone since it was listed.
            Ok(None) => {}
            // Still there, but unreadable.
            Err(()) => whole = false,
        }
    }
    (out, whole)
}

/// Every child of `pid`, whichever of its threads forked it: each task
/// lists only its own (`/proc/<pid>/task/<tid>/children`). `Err` when a
/// file of a process or thread that still exists could not be read; a
/// process or thread that is gone has no children.
#[cfg(target_os = "linux")]
fn read_children_checked(pid: u32) -> Result<Vec<u32>, ()> {
    use std::io::ErrorKind;
    let gone = |error: &std::io::Error| {
        matches!(error.kind(), ErrorKind::NotFound) || error.raw_os_error() == Some(3)
    };
    let dir = match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(dir) => dir,
        Err(error) if gone(&error) => return Ok(Vec::new()),
        Err(_) => return Err(()),
    };
    let mut children = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|_| ())?;
        let tid = entry.file_name();
        let task = format!("/proc/{pid}/task/{}", tid.to_string_lossy());
        match std::fs::read_to_string(format!("{task}/children")) {
            Ok(listed) => children.extend(
                listed
                    .split_whitespace()
                    .filter_map(|s| s.parse::<u32>().ok()),
            ),
            // The thread exited. A kernel without the `children` file has
            // the thread but not the file: that is unreadable, not empty.
            Err(error) if gone(&error) && !std::path::Path::new(&task).exists() => {}
            Err(_) => return Err(()),
        }
    }
    children.sort_unstable();
    children.dedup();
    Ok(children)
}

/// [`read_proc`] for evidence: `Ok(None)` when the process is gone, `Err`
/// when it still exists but its status, name or command line could not be
/// read (the display fallbacks [`read_proc`] uses are not evidence).
#[cfg(target_os = "linux")]
fn read_proc_checked(pid: u32, depth: u8) -> Result<Option<ProcessInfo>, ()> {
    let exists = || std::path::Path::new(&format!("/proc/{pid}")).exists();
    let read = |file: &str| std::fs::read(format!("/proc/{pid}/{file}"));
    let (status, comm, cmdline) = match (read("status"), read("comm"), read("cmdline")) {
        (Ok(status), Ok(comm), Ok(cmdline)) => (status, comm, cmdline),
        _ if exists() => return Err(()),
        _ => return Ok(None),
    };
    let parent_pid = parse_ppid(&String::from_utf8_lossy(&status)).ok_or(())?;
    Ok(Some(ProcessInfo {
        pid,
        parent_pid,
        depth,
        comm: String::from_utf8_lossy(&comm).trim().to_string(),
        cmdline: cmdline
            .split(|b| *b == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part))
            .collect::<Vec<_>>()
            .join(" "),
    }))
}

#[cfg(target_os = "linux")]
pub(crate) fn read_proc(pid: u32, depth: u8) -> Option<ProcessInfo> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let parent_pid = parse_ppid(&status)?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map_or_else(|| pid.to_string(), |s| s.trim().to_string());
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|bytes| {
            bytes
                .split(|b| *b == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    Some(ProcessInfo {
        pid,
        parent_pid,
        depth,
        comm,
        cmdline,
    })
}

#[cfg(target_os = "linux")]
fn parse_ppid(status: &str) -> Option<u32> {
    status.lines().find_map(|line| {
        line.strip_prefix("PPid:")
            .and_then(|rest| rest.trim().parse().ok())
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_descendants(root: u32) -> Vec<ProcessInfo> {
    crate::process_snapshot::read_current_process_table().descendants(root, MAX_DEPTH, MAX_NODES)
}

/// [`read_descendants`] of `root` in an already read `table`.
#[cfg(not(target_os = "linux"))]
pub(crate) fn descendants_in(
    table: &crate::process_snapshot::ProcessTable,
    root: u32,
) -> Vec<ProcessInfo> {
    table.descendants(root, MAX_DEPTH, MAX_NODES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent with no child processes is a workload, not an empty pane.
    #[test]
    fn a_lone_agent_process_is_not_an_empty_workload() {
        let summary = summarize(vec![proc(20, 10, 1, "claude", "claude --resume x")]);
        assert!(summary.primary_pid.is_some());
        assert!(!summary.is_empty());
        assert!(WorkloadSummary::default().is_empty());
    }

    fn proc(pid: u32, parent_pid: u32, depth: u8, comm: &str, cmdline: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            depth,
            comm: comm.into(),
            cmdline: cmdline.into(),
        }
    }

    /// Only a walk of every pane, from a table that was read, says what is
    /// not running.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_walk_is_complete_only_when_every_pane_was_read() {
        let sid = "01a132f3-21bf-76e3-822e-e84855943e22";
        let pane = |id: &str, pid: u32| -> PaneInfo {
            serde_json::from_value(serde_json::json!({
                "pane_id": id, "session": "tend", "window_index": "0", "pane_index": "0",
                "tty": "", "current_command": "codex", "title": "", "pane_pid": pid,
                "socket": "default"
            }))
            .unwrap()
        };
        let source = |processes: Vec<ProcessInfo>| ProcessSource {
            table: crate::process_snapshot::ProcessTable::from_processes(processes),
        };
        let table = || {
            vec![
                proc(10, 1, 0, "zsh", "-zsh"),
                proc(
                    20,
                    10,
                    1,
                    "codex",
                    &format!("codex --no-daemon resume {sid}"),
                ),
            ]
        };
        let scan = scan_codex_panes_with(&[pane("%8", 10)], &source(table()));
        assert!(scan.complete);
        assert_eq!(
            scan.identities
                .get(&PaneKey::of(&pane("%8", 10)))
                .map(|i| i.resumed.as_slice()),
            Some([sid.to_owned()].as_slice())
        );
        // A failed `ps`.
        assert!(
            !scan_codex_panes_with(
                &[pane("%8", 10)],
                &ProcessSource {
                    table: crate::process_snapshot::ProcessTable::default()
                }
            )
            .complete
        );
        // A pane with no pid, or whose root is not in the table.
        assert!(
            !scan_codex_panes_with(&[pane("%8", 10), pane("%9", 0)], &source(table())).complete
        );
        assert!(
            !scan_codex_panes_with(&[pane("%8", 10), pane("%9", 99)], &source(table())).complete
        );
        // A walk cut off at its node limit, or past its depth limit.
        let mut wide = table();
        wide.extend((0..300).map(|i| proc(1000 + i, 10, 1, "sleep", "sleep 1")));
        assert!(!scan_codex_panes_with(&[pane("%8", 10)], &source(wide)).complete);
        let mut deep = vec![proc(10, 1, 0, "zsh", "-zsh")];
        deep.extend((0..12).map(|i| {
            proc(
                100 + i,
                if i == 0 { 10 } else { 99 + i },
                1,
                "sh",
                "sh -c x",
            )
        }));
        deep.push(proc(200, 111, 1, "codex", &format!("codex resume {sid}")));
        let scan = scan_codex_panes_with(&[pane("%8", 10)], &source(deep));
        assert!(!scan.complete, "depth truncation is not a whole walk");
        // The same pane id on two sockets: each keeps its own evidence.
        let other = || -> PaneInfo {
            serde_json::from_value(serde_json::json!({
                "pane_id": "%8", "session": "x", "window_index": "0", "pane_index": "0",
                "tty": "", "current_command": "claude", "title": "", "pane_pid": 50,
                "socket": "/tmp/tmux-501/work"
            }))
            .unwrap()
        };
        let mut both = table();
        both.push(proc(50, 1, 0, "zsh", "-zsh"));
        both.push(proc(60, 50, 1, "claude", "claude --continue"));
        let scan = scan_codex_panes_with(&[pane("%8", 10), other()], &source(both));
        let here = &scan.identities[&PaneKey::of(&pane("%8", 10))];
        let there = &scan.identities[&PaneKey::of(&other())];
        assert_eq!((here.resumed.len(), here.unnamed), (1, 0));
        assert_eq!((there.resumed.len(), there.unnamed), (0, 1));
    }

    #[test]
    fn a_shim_is_one_session_and_any_other_nested_agent_is_its_own() {
        let sid = "01a132f3-21bf-76e3-822e-e84855943e22";
        let named = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(
                20,
                10,
                1,
                "aas",
                &format!("aas exec x@codex -- resume {sid}"),
            ),
            proc(
                21,
                20,
                2,
                "codex",
                &format!("codex --no-daemon resume {sid}"),
            ),
            proc(22, 21, 3, "node_repl", "node_repl"),
        ]);
        assert_eq!(
            (named.agent, named.resumed.len(), named.unnamed),
            (true, 1, 0)
        );
        let beside = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(21, 10, 1, "codex", &format!("codex resume {sid}")),
            proc(30, 10, 1, "claude", "claude --continue"),
        ]);
        assert_eq!(beside.unnamed, 1);
        // Nested under it, through a shell, it is still its own session.
        let nested = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(21, 10, 1, "codex", &format!("codex resume {sid}")),
            proc(25, 21, 2, "zsh", "zsh -c claude"),
            proc(30, 25, 3, "claude", "claude --continue"),
        ]);
        assert_eq!((nested.resumed.len(), nested.unnamed), (1, 1));
        // The same flags through a shell: two sessions, each its own pick.
        let twice = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(20, 10, 1, "claude", "claude --continue"),
            proc(25, 20, 2, "zsh", "zsh -c 'cd /other && claude --continue'"),
            proc(30, 25, 3, "claude", "claude --continue"),
        ]);
        assert_eq!(twice.unnamed, 2);
        // A shim and the binary it starts, with the same arguments: one.
        let shim = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(20, 10, 1, "codex", "codex --yolo"),
            proc(21, 20, 2, "codex", "/opt/codex/bin/codex --yolo"),
        ]);
        assert_eq!(shim.unnamed, 1);
        // A child with other arguments (a subagent run): its own session.
        let sub = pane_identities(&[
            proc(10, 1, 0, "zsh", "-zsh"),
            proc(20, 10, 1, "codex", "codex --yolo"),
            proc(21, 20, 2, "codex", "codex exec do-it"),
        ]);
        assert_eq!(sub.unnamed, 2);
    }

    #[test]
    fn every_explicit_resume_is_listed_whatever_its_hooks_or_pane_count() {
        let codex = "01a132f3-21bf-76e3-822e-e84855943e22";
        let claude = "55115377-0000-4000-8000-000000000000";
        let isolated = vec![proc(
            20,
            10,
            1,
            "codex",
            &format!("codex --no-daemon --yolo resume {codex}"),
        )];
        let twice = vec![proc(
            30,
            11,
            1,
            "claude",
            &format!("claude --resume {claude}"),
        )];
        let fresh = vec![proc(40, 12, 1, "codex", "codex --yolo")];
        let resumed = resumed_sessions(
            [
                isolated.as_slice(),
                twice.as_slice(),
                twice.as_slice(),
                fresh.as_slice(),
            ]
            .into_iter(),
        );
        assert_eq!(
            resumed.into_iter().collect::<Vec<_>>(),
            [codex.to_owned(), claude.to_owned()]
        );
    }

    #[test]
    fn resume_binding_deduplicates_wrappers_and_rejects_multiple_endpoints() {
        let sid = "01a0f231-262f-77a3-9d42-b36d645fefcd";
        let mut a: PaneInfo = serde_json::from_value(serde_json::json!({
            "pane_id":"%181", "session":"tend", "window_index":"0", "pane_index":"0",
            "tty":"", "current_command":"codex", "title":"", "pane_pid":10,
            "socket":"default"
        }))
        .unwrap();
        let processes = vec![
            proc(20, 10, 1, "codex", &format!("codex resume {sid}")),
            proc(21, 20, 2, "codex", &format!("codex resume {sid}")),
        ];
        let bindings = unique_resume_bindings(std::iter::once((&a, processes.clone())));
        assert!(processes.iter().all(is_interactive_codex));
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].session_id, sid);
        assert_eq!(bindings[0].kind, AgentKind::Codex);
        let b = a.clone();
        a.socket = Some("other-server".into());
        assert!(
            unique_resume_bindings([(&a, processes.clone()), (&b, processes)].into_iter())
                .is_empty(),
            "same session in multiple servers must remain ambiguous"
        );
    }

    #[test]
    fn isolated_codex_panes_cannot_host_paneless_shared_threads() {
        let isolated = proc(20, 10, 1, "codex", "codex --no-daemon --yolo");
        assert!(isolated_codex_host(std::slice::from_ref(&isolated)));
        let shared = proc(15, 10, 0, "codex", "codex --yolo");
        assert!(!isolated_codex_host(&[shared, isolated]));
    }

    #[test]
    fn interactive_codex_excludes_headless_and_server_runs() {
        for (comm, args, expected) in [
            ("codex", "/vendor/bin/codex --yolo Do this", true),
            (
                "codex",
                "codex --yolo resume 01a0f231-262f-77a3-9d42-b36d645fefcd",
                true,
            ),
            ("codex", "codex", true),
            (
                "codex",
                "/x/bin/codex app-server --listen unix:// --managed-daemon",
                false,
            ),
            ("codex", "codex exec explain this", false),
            ("codex", "codex --yolo e explain", false),
            ("zsh", "zsh -c codex", false),
        ] {
            assert_eq!(
                is_interactive_codex(&proc(20, 10, 1, comm, args)),
                expected,
                "{args}"
            );
        }
    }

    #[test]
    fn claude_resume_identity_reads_the_explicit_session_flags() {
        let sid = "9e48c8ff-1b70-4a3e-9f32-1581a05f0f75";
        for args in [
            format!("/home/u/.local/bin/claude --dangerously-skip-permissions --resume {sid}"),
            format!("claude -r {sid}"),
            format!("claude --resume={sid}"),
            format!("claude --fork-session --session-id {sid}"),
        ] {
            assert_eq!(
                resume_id(&proc(30, 10, 2, "claude", &args)),
                Some(sid),
                "{args}"
            );
        }
        for (comm, args) in [
            // The launch wrapper carries the same argv but is not the agent.
            ("aas", format!("aas exec e-ed@claude -- --resume {sid}")),
            ("claude", "claude --resume".into()),
            ("claude", "claude --resume my-session-title".into()),
            ("claude", "claude --dangerously-skip-permissions".into()),
        ] {
            assert_eq!(resume_id(&proc(30, 10, 2, comm, &args)), None, "{args}");
        }
    }

    #[test]
    fn resume_identity_requires_codex_and_explicit_uuid() {
        let sid = "01a0f231-262f-77a3-9d42-b36d645fefcd";
        let cli = proc(
            20,
            10,
            1,
            "/bin/codex",
            &format!("codex --yolo resume {sid}"),
        );
        assert_eq!(codex_resume_id(&cli), Some(sid));
        for (comm, args) in [
            ("zsh", format!("zsh -c codex resume {sid}")),
            ("codex", "codex resume --last".into()),
            ("codex", "codex resume session-title".into()),
            ("codex", "codex app-server --listen unix://".into()),
            ("codex", format!("codex exec explain resume {sid}")),
        ] {
            assert_eq!(codex_resume_id(&proc(20, 10, 1, comm, &args)), None);
        }
    }

    #[test]
    fn summarize_counts_shell_and_child_process_under_primary_agent() {
        let summary = summarize(vec![
            proc(20, 10, 1, "claude", "claude"),
            proc(30, 20, 2, "zsh", "zsh -c run"),
            proc(31, 30, 3, "python3", "python3 -m tests.manual"),
        ]);
        assert_eq!(summary.primary_pid, Some(20));
        assert_eq!(summary.shell_count, 1);
        assert_eq!(summary.subagent_count, 0);
        assert_eq!(summary.process_count, 2);
        assert_eq!(
            summary
                .preview
                .iter()
                .map(|p| p.command.as_str())
                .collect::<Vec<_>>(),
            vec!["zsh", "python3"]
        );
    }

    #[test]
    fn summarize_excludes_mcp_helpers_from_visible_workload() {
        let summary = summarize(vec![
            proc(20, 10, 1, "claude", "claude"),
            proc(
                30,
                20,
                2,
                "sh",
                "sh -c \"playwright-mcp\" --browser chromium",
            ),
        ]);
        assert_eq!(summary.primary_pid, Some(20));
        assert_eq!(summary.helper_count, 1);
        assert_eq!(summary.shell_count, 0);
        assert_eq!(summary.process_count, 0);
        assert!(summary.preview.is_empty());
    }

    #[test]
    fn summarize_counts_nested_agent_as_subagent() {
        let summary = summarize(vec![
            proc(20, 10, 1, "claude", "claude"),
            proc(
                40,
                20,
                2,
                "2.1.177",
                "/home/u/.local/share/claude/versions/2.1.177 --session-id s --fork-session",
            ),
        ]);
        assert_eq!(summary.primary_pid, Some(20));
        assert_eq!(summary.subagent_count, 1);
        assert_eq!(summary.process_count, 1);
        assert_eq!(summary.preview[0].command, "claude");
    }
}
