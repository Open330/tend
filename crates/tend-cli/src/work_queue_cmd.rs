//! `tend work queue`: prompts tendd sends to an agent once it is free, or
//! Work it starts once the queue's throttle allows. tendd owns the queue
//! (`work_queue_v1`); this is a thin client.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;
use tend::ipc::Client;
use tend::work_queue::{
    QueueAddRequest, QueueEditRequest, QueueItem, QueueItemView, QueueSnapshot, QueueState,
    QueueTarget, QueueTargetSpec, WaitReason, WORK_QUEUE_CAPABILITY,
};

#[derive(Debug, Subcommand)]
pub enum QueueCmd {
    /// Show the queue in order, with why each waiting item has not gone.
    #[command(alias = "ls")]
    List(ListArgs),
    /// Queue a prompt for an agent (`--agent`), for one agent of a Work
    /// (`--work`), or as a new Work (`--new`).
    Add(AddArgs),
    /// Change a waiting item's prompt or title.
    Edit(EditArgs),
    /// Move a waiting item to a place in the queue (0 = next).
    Move(MoveArgs),
    /// Cancel a waiting item; it stays listed as cancelled.
    Cancel(ItemArgs),
    /// Forget an item. One that may already have reached its agent needs
    /// `--ack-duplicate-risk`.
    #[command(alias = "remove")]
    Rm(RmArgs),
    /// Send or start an item now, ahead of the queue, if its agent is free.
    RunNow(RunNowArgs),
    /// Settle an uncertain item: `--sent` (it arrived) or `--not-sent`
    /// (queue it again).
    Resolve(ResolveArgs),
    /// Forget every finished item (sent, failed, cancelled).
    Clear(JsonArgs),
}

#[derive(Debug, Args)]
pub struct JsonArgs {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Include finished items.
    #[arg(long, short = 'a')]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// The prompt to send (or the new Work's request). `-` reads stdin.
    prompt: String,
    /// Target agent: a session id or a pane (`%12`).
    #[arg(long, conflicts_with_all = ["work", "new"])]
    agent: Option<String>,
    /// Target one agent of a Work: `WORKSPACE/WORK` or `WORKSPACE/WORK@ALIAS`.
    #[arg(long, conflicts_with = "new")]
    work: Option<String>,
    /// Start a new Work with this prompt when the queue's throttle allows.
    #[arg(long)]
    new: bool,
    /// New Work: a short name that shapes its id (the id is always new).
    #[arg(long, requires = "new")]
    name: Option<String>,
    /// New Work: the directory to start in. Defaults to the current one.
    #[arg(long, requires = "new")]
    cwd: Option<PathBuf>,
    /// New Work: the workspace. Defaults to the directory's name.
    #[arg(long, requires = "new")]
    workspace: Option<String>,
    /// New Work: the pipeline. Defaults to the route's.
    #[arg(long, requires = "new")]
    pipeline: Option<String>,
    /// A title for the queue list.
    #[arg(long)]
    title: Option<String>,
    /// Idempotency key (a UUID). The same key with the same item returns
    /// the item already added instead of adding another; tendd remembers
    /// the last 1000 keys for 7 days. Generated when omitted and printed.
    #[arg(long)]
    request_key: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct ItemArgs {
    /// Item id (`q-…`; a unique prefix, with or without `q-`, is enough).
    id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct EditArgs {
    id: String,
    #[arg(long)]
    prompt: Option<String>,
    /// New title; an empty one clears it.
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct MoveArgs {
    id: String,
    /// 0-based place among the items not finished.
    to: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct RmArgs {
    id: String,
    /// Remove it even though its text may already be in the agent's input;
    /// the queue may then type to that agent again.
    #[arg(long)]
    ack_duplicate_risk: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct RunNowArgs {
    id: String,
    /// Run an uncertain item again, accepting that it may arrive twice.
    #[arg(long)]
    allow_duplicate: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // clap flags
pub struct ResolveArgs {
    id: String,
    /// It reached the agent (or the Work started).
    #[arg(
        long,
        conflicts_with = "not_sent",
        required_unless_present = "not_sent"
    )]
    sent: bool,
    /// It did not; queue it again.
    #[arg(long)]
    not_sent: bool,
    /// Required with `--not-sent`: if it did arrive, it arrives twice.
    #[arg(long)]
    ack_duplicate_risk: bool,
    #[arg(long)]
    json: bool,
}

pub async fn run(cmd: QueueCmd, client: &Client) -> Result<()> {
    require_queue(client).await?;
    match cmd {
        QueueCmd::List(args) => {
            let snapshot = client.work_queue_list(args.all).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                print!("{}", render(&snapshot));
            }
            Ok(())
        }
        QueueCmd::Add(args) => add(args, client).await,
        QueueCmd::Edit(args) => {
            let (id, revision) = find(client, &args.id).await?;
            let edit = QueueEditRequest {
                title: args.title,
                prompt: args.prompt.map(read_prompt).transpose()?,
                target: None,
            };
            let item = client.work_queue_edit(&id, Some(revision), &edit).await?;
            done(&item, args.json, "edited")
        }
        QueueCmd::Move(args) => {
            let (id, revision) = find(client, &args.id).await?;
            let item = client.work_queue_move(&id, Some(revision), args.to).await?;
            done(&item, args.json, "moved")
        }
        QueueCmd::Cancel(args) => {
            let (id, revision) = find(client, &args.id).await?;
            let item = client.work_queue_cancel(&id, Some(revision)).await?;
            done(&item, args.json, "cancelled")
        }
        QueueCmd::Rm(args) => {
            let (id, _) = find(client, &args.id).await?;
            let item = client
                .work_queue_remove(&id, args.ack_duplicate_risk)
                .await?;
            done(&item, args.json, "removed")
        }
        QueueCmd::RunNow(args) => {
            let (id, revision) = find(client, &args.id).await?;
            let item = client
                .work_queue_run_now(&id, Some(revision), args.allow_duplicate)
                .await?;
            done(&item, args.json, "run")
        }
        QueueCmd::Resolve(args) => {
            if args.not_sent && !args.ack_duplicate_risk {
                bail!(
                    "--not-sent queues it again; if it did reach the agent it arrives twice. \
                     Add --ack-duplicate-risk to go ahead"
                );
            }
            let (id, _) = find(client, &args.id).await?;
            let item = client
                .work_queue_resolve(&id, args.sent, args.ack_duplicate_risk)
                .await?;
            done(&item, args.json, "resolved")
        }
        QueueCmd::Clear(args) => {
            let count = client.work_queue_clear().await?;
            if args.json {
                println!("{}", serde_json::json!({ "cleared": count }));
            } else {
                println!("cleared {count} finished item(s)");
            }
            Ok(())
        }
    }
}

async fn require_queue(client: &Client) -> Result<()> {
    let hello = client
        .hello(std::time::Duration::from_secs(2))
        .await
        .context("reaching tendd")?;
    if !hello
        .capabilities
        .iter()
        .any(|c| c == WORK_QUEUE_CAPABILITY)
    {
        bail!("this tendd has no Work queue; restart it after upgrading tend");
    }
    Ok(())
}

fn read_prompt(prompt: String) -> Result<String> {
    if prompt == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .context("reading the prompt from stdin")?;
        Ok(text)
    } else {
        Ok(prompt)
    }
}

/// Parse `--work WORKSPACE/WORK[@ALIAS]`.
fn work_target(spec: &str) -> Result<QueueTargetSpec> {
    let (path, alias) = match spec.split_once('@') {
        Some((path, alias)) => (path, Some(alias.to_string())),
        None => (spec, None),
    };
    let Some((workspace, work)) = path.split_once('/') else {
        bail!("--work takes WORKSPACE/WORK or WORKSPACE/WORK@ALIAS");
    };
    if workspace.trim().is_empty() || work.trim().is_empty() {
        bail!("--work takes WORKSPACE/WORK or WORKSPACE/WORK@ALIAS");
    }
    Ok(QueueTargetSpec::Work {
        workspace: workspace.trim().into(),
        work: work.trim().into(),
        alias,
    })
}

fn add_target(args: &AddArgs) -> Result<QueueTargetSpec> {
    if args.new {
        let cwd = match &args.cwd {
            Some(cwd) => tend::paths::canonicalize(cwd)
                .with_context(|| format!("resolving {}", cwd.display()))?,
            None => std::env::current_dir().context("reading the current directory")?,
        };
        return Ok(QueueTargetSpec::NewWork {
            work: args.name.clone(),
            workspace: args.workspace.clone(),
            cwd: Some(cwd),
            pipeline: args.pipeline.clone(),
            external: None,
            skill: None,
            context: None,
        });
    }
    if let Some(work) = &args.work {
        return work_target(work);
    }
    let Some(agent) = &args.agent else {
        bail!("name a target: --agent SESSION|%PANE, --work WORKSPACE/WORK[@ALIAS], or --new");
    };
    Ok(if agent.starts_with('%') || agent.contains(":%") {
        QueueTargetSpec::Agent {
            session_id: None,
            pane: Some(agent.clone()),
            socket: None,
            label: None,
        }
    } else {
        QueueTargetSpec::Agent {
            session_id: Some(agent.clone()),
            pane: None,
            socket: None,
            label: None,
        }
    })
}

async fn add(args: AddArgs, client: &Client) -> Result<()> {
    let target = add_target(&args)?;
    let request_key = args
        .request_key
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let request = QueueAddRequest {
        request_key: request_key.clone(),
        title: args.title.clone(),
        prompt: read_prompt(args.prompt.clone())?,
        target,
    };
    let item = client.work_queue_add(&request).await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "item": item,
                "request_key": request_key,
            }))?
        );
    } else {
        println!("queued {} ({})", item.id, target_text(&item.target));
        println!("request key {request_key} (pass --request-key to retry safely)");
    }
    Ok(())
}

/// Resolve an id or unique prefix to the full id and its revision.
async fn find(client: &Client, id: &str) -> Result<(String, u64)> {
    let snapshot = client.work_queue_list(true).await?;
    let matches: Vec<&QueueItemView> = snapshot
        .items
        .iter()
        .filter(|view| {
            view.item.id == id
                || view.item.id.starts_with(id)
                || view.item.id.trim_start_matches("q-").starts_with(id)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok((one.item.id.clone(), one.item.revision)),
        [] => bail!("no queued item {id}"),
        _ => match matches.iter().find(|view| view.item.id == id) {
            Some(exact) => Ok((exact.item.id.clone(), exact.item.revision)),
            None => bail!("{id} matches {} items; give more of the id", matches.len()),
        },
    }
}

fn done(item: &QueueItem, json: bool, verb: &str) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(item)?);
    } else {
        println!("{verb} {}: {}", item.id, state_text(&item.state));
    }
    Ok(())
}

fn target_text(target: &QueueTarget) -> String {
    match target {
        QueueTarget::Agent { session_id, label } => label
            .clone()
            .unwrap_or_else(|| format!("agent {}", short(session_id))),
        QueueTarget::Work { identity, alias } => match alias {
            Some(alias) => format!("{}@{alias}", identity.key()),
            None => identity.key(),
        },
        QueueTarget::NewWork { request } => format!(
            "new Work {}/{}",
            request.workspace.as_deref().unwrap_or("?"),
            request.work
        ),
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn state_text(state: &QueueState) -> String {
    match state {
        QueueState::Waiting => "waiting".into(),
        QueueState::Dispatching { .. } => "sending now".into(),
        QueueState::Dispatched { .. } => "sent".into(),
        QueueState::Uncertain { reason, .. } => format!("uncertain: {reason}"),
        QueueState::Failed { detail, .. } => format!("failed: {detail}"),
        QueueState::Cancelled { .. } => "cancelled".into(),
    }
}

/// The same short wording the daemon uses for a run-now refusal.
fn wait_text(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::AgentBusy => "the agent is working",
        WaitReason::AgentNeedsYou => "the agent is waiting for you",
        WaitReason::AgentRateLimited => "the agent is at its usage limit",
        WaitReason::AgentError => "the agent is in an error state",
        WaitReason::AgentMissing => "the agent is not running",
        WaitReason::AgentAmbiguous => "the agent shows on more than one pane",
        WaitReason::WorkAmbiguous => "the Work has several agents; name one",
        WaitReason::Draining => "input paused by a worker drain",
        WaitReason::Reserved => "an earlier queued prompt is not picked up yet",
        WaitReason::Behind => "behind an earlier item for the same agent",
        WaitReason::NoSlot => "too many agents are busy",
        WaitReason::Launching => "another queued Work is starting",
        WaitReason::RetryLater => "trying again shortly",
        WaitReason::StoreUnsaved => "tendd could not save the queue",
    }
}

fn render(snapshot: &QueueSnapshot) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if let Some(broken) = &snapshot.broken {
        let _ = writeln!(
            out,
            "the queue file could not be read ({broken}); it is read-only"
        );
    }
    if snapshot.unsaved {
        let _ = writeln!(
            out,
            "tendd could not save the queue; nothing new goes until it can"
        );
    }
    if snapshot.items.is_empty() {
        out.push_str("the queue is empty\n");
        return out;
    }
    for view in &snapshot.items {
        let item = &view.item;
        let place = view
            .position
            .map_or_else(|| "  -".to_string(), |p| format!("{p:>3}"));
        let status = match (&item.state, view.wait) {
            (QueueState::Waiting, Some(reason)) => format!("waiting: {}", wait_text(reason)),
            (state, _) => state_text(state),
        };
        let title = item
            .title
            .clone()
            .unwrap_or_else(|| item.prompt.lines().next().unwrap_or_default().to_string());
        let title: String = title.chars().take(60).collect();
        let _ = writeln!(
            out,
            "{place}  {}  {}  {title}\n       {status}",
            short(item.id.trim_start_matches("q-")),
            target_text(&item.target),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_targets_parse_with_and_without_an_alias() {
        assert!(matches!(
            work_target("web/ENG-1@impl").unwrap(),
            QueueTargetSpec::Work { workspace, work, alias: Some(alias) }
                if workspace == "web" && work == "ENG-1" && alias == "impl"
        ));
        assert!(matches!(
            work_target("web/ENG-1").unwrap(),
            QueueTargetSpec::Work { alias: None, .. }
        ));
        for bad in ["ENG-1", "/ENG-1", "web/", "@impl"] {
            assert!(work_target(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_empty_queue_says_so() {
        let snapshot = QueueSnapshot {
            items: Vec::new(),
            revision: 0,
            evaluated_at: None,
            broken: None,
            unsaved: false,
            fences: Vec::new(),
        };
        assert_eq!(render(&snapshot), "the queue is empty\n");
    }
}
