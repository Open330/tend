//! Deterministic tmux agent launcher shared by the CLI and MCP tool.
//!
//! This deliberately exposes an allowlist of known agent CLIs instead of an
//! arbitrary shell command. Agents can create a worker pane without spending
//! a model turn reconstructing tmux syntax, while callers still get a narrow,
//! predictable operation.

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use serde::Serialize;
use std::path::{Path, PathBuf};

use tend::backend::HostKind;
use tend::ipc::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AgentProgram {
    Claude,
    Codex,
    /// The Antigravity CLI. Named for its binary (`agy`) rather than the
    /// product; `antigravity` stays accepted
    /// as an alias on both the CLI flag and [`AgentProgram::parse`].
    #[value(name = "agy", alias = "antigravity")]
    #[serde(rename = "agy")]
    Antigravity,
    Opencode,
}

impl AgentProgram {
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude_code" | "claude-code" => Ok(Self::Claude),
            "codex" | "cx" => Ok(Self::Codex),
            "agy" | "antigravity" => Ok(Self::Antigravity),
            "opencode" => Ok(Self::Opencode),
            other if tend::agent_control::is_retired_agent_program(other) => {
                Err(tend::agent_control::RETIRED_GEMINI_NOTE.into())
            }
            _ => Err(format!(
                "unknown agent {value:?}; expected claude, codex, agy, or opencode"
            )),
        }
    }

    /// The provider's CLI name, and the `[agent.<program>]` config key.
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "agy",
            Self::Opencode => "opencode",
        }
    }

    #[cfg(test)]
    fn launch_argv(self, options: &[String], prompt: Option<&str>) -> Vec<String> {
        self.native_launch(options, prompt).argv()
    }

    #[cfg(test)]
    fn launch_command(self, options: &[String], prompt: Option<&str>) -> String {
        tend::shell::Dialect::Posix.command_line(&self.launch_argv(options, prompt))
    }

    fn native_launch(self, options: &[String], prompt: Option<&str>) -> NativeLaunch {
        self.permission_launch(options, prompt, true)
    }

    fn permission_launch(
        self,
        options: &[String],
        prompt: Option<&str>,
        bypass: bool,
    ) -> NativeLaunch {
        let mut args = if bypass {
            match self {
                Self::Claude | Self::Antigravity => {
                    vec!["--dangerously-skip-permissions".into()]
                }
                Self::Codex => vec!["--yolo".into()],
                Self::Opencode => Vec::new(),
            }
        } else {
            Vec::new()
        };
        args.extend(options.iter().cloned());
        if let Some(prompt) = prompt {
            match self {
                Self::Claude | Self::Codex => args.push(prompt.into()),
                Self::Antigravity => {
                    args.push("-i".into());
                    args.push(prompt.into());
                }
                Self::Opencode => {
                    args.push("--prompt".into());
                    args.push(prompt.into());
                }
            }
        }
        if self == Self::Codex {
            // Never come up on Codex's update menu or directory question
            // (see `tend::codex_launch`).
            args = tend::codex_launch::with_startup_overrides(&args);
        }
        NativeLaunch {
            command: self.label().into(),
            args,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLaunch {
    command: String,
    args: Vec<String>,
}

impl NativeLaunch {
    /// This launch, run on the account `routed` chose. A launch routing
    /// skipped or never asked for runs as built.
    fn on_account(
        self,
        routed: Option<&std::result::Result<tend::swap_api::RoutedLaunch, String>>,
    ) -> Self {
        match routed {
            Some(Ok(routed)) => {
                let mut argv = routed.argv(self.args).into_iter();
                Self {
                    command: argv.next().expect("aas argv has a program"),
                    args: argv.collect(),
                }
            }
            _ => self,
        }
    }

    fn argv(self) -> Vec<String> {
        std::iter::once(self.command).chain(self.args).collect()
    }
}

/// Settle `[agent.<program>].account` for one launch, before anything is
/// created. A skipped route is reported, not fatal: the agent still starts.
/// A capped one is fatal: the agent would only sit on the limit screen.
/// `passed_over` are accounts this launch already saw at their limit.
fn route_account(
    route: Option<&tend::swap_api::AccountRoute>,
    passed_over: &[String],
) -> Result<Option<std::result::Result<tend::swap_api::RoutedLaunch, String>>> {
    let Some(route) = route else {
        return Ok(None);
    };
    match crate::account_module::decide(route, passed_over) {
        Ok(routed) => Ok(Some(Ok(routed))),
        Err(tend::swap_api::NotRouted::Bare(reason)) => {
            eprintln!("tend: starting without an account: {reason}");
            Ok(Some(Err(reason)))
        }
        Err(tend::swap_api::NotRouted::Capped(reason)) => {
            Err(not_started(format!("did not start: {reason}"), None))
        }
    }
}

/// Execution host selection for the user-facing agent launcher.
///
/// The first native slice deliberately exposes only hosts that can create a
/// new surface today. Observation-only pane backends remain available to the
/// daemon and will join this enum as their launch lifecycle lands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum LaunchHost {
    /// Use the multiplexer when this shell is in a real tmux or rmux pane;
    /// otherwise use tend's PTY.
    #[default]
    Auto,
    /// Spawn a tend-owned PTY session through tendd.
    Native,
    /// Use the existing managed tmux launcher.
    Tmux,
}

impl LaunchHost {
    fn resolve(self, detected: Option<HostKind>) -> Self {
        match self {
            // rmux speaks tmux's commands, so the same launcher places the
            // agent in an rmux pane — the only multiplexer rmux on Windows has.
            Self::Auto if matches!(detected, Some(HostKind::Tmux | HostKind::Rmux)) => Self::Tmux,
            Self::Auto => Self::Native,
            explicit => explicit,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Pane,
    Window,
    Session,
}

impl Placement {
    pub fn parse(value: Option<&str>) -> std::result::Result<Self, String> {
        match value.unwrap_or("pane").trim().to_ascii_lowercase().as_str() {
            "pane" | "split" => Ok(Self::Pane),
            "window" => Ok(Self::Window),
            "session" => Ok(Self::Session),
            other => Err(format!(
                "unknown placement {other:?}; expected pane, window, or session"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum SplitDirection {
    /// Split along the pane's longer side, which is what a person reaching
    /// for the divider would do. The default: a line-up that keeps growing
    /// stays readable instead of collapsing into thin columns.
    #[default]
    Auto,
    Right,
    Down,
}

impl SplitDirection {
    pub fn parse(value: Option<&str>) -> std::result::Result<Self, String> {
        match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Ok(Self::Auto),
            "right" | "horizontal" => Ok(Self::Right),
            "down" | "vertical" => Ok(Self::Down),
            other => Err(format!(
                "unknown direction {other:?}; expected auto, right, or down"
            )),
        }
    }

    /// The tmux flag for this split, resolving `auto` against the target
    /// pane's shape. A cell is about twice as tall as it is wide, so the
    /// comparison is on the pane's visual proportions rather than its rows
    /// and columns.
    #[must_use]
    pub fn tmux_flag(self, target: &str) -> &'static str {
        match self {
            Self::Right => "-h",
            Self::Down => "-v",
            // A cell is about twice as tall as it is wide, so a pane is
            // "wider than tall" when its columns exceed twice its rows.
            // Without a measurement — no tmux server, unknown target — the
            // historical right split stands.
            Self::Auto => match pane_shape(target) {
                Some((columns, rows)) if columns < rows.saturating_mul(2) => "-v",
                _ => "-h",
            },
        }
    }
}

/// `#{pane_width}x#{pane_height}` for a target pane, or `None` when tmux
/// cannot answer (no server, unknown target).
fn pane_shape(target: &str) -> Option<(u32, u32)> {
    let mut command = crate::mux_control::ambient_command_with_args(&[
        "display",
        "-p",
        "-t",
        target,
        "#{pane_width}x#{pane_height}",
    ])
    .ok()?;
    let output = crate::subprocess::output(&mut command).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (width, height) = text.trim().split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

#[derive(Debug, clap::Args)]
pub struct StartArgs {
    /// Primary protects human work; worker explicitly accepts peer work.
    #[arg(long, default_value = "primary")]
    pub purpose: tend::collaboration::LaunchPurpose,
    /// Known agent CLI to launch. codex expands the local cx profile (codex --yolo).
    #[arg(long, value_enum)]
    pub agent: AgentProgram,
    /// Execution host. Auto uses the multiplexer inside tmux or rmux and a
    /// tend-owned PTY elsewhere.
    #[arg(long, value_enum, default_value = "auto")]
    pub host: LaunchHost,
    /// Create a split pane (default), window, or independent session.
    #[arg(long, value_enum, default_value = "pane")]
    pub placement: Placement,
    /// tmux pane/window target. Defaults to `TMUX_PANE` (ignored under a shared Codex app-server); unused for session placement.
    #[arg(long)]
    pub target: Option<String>,
    /// Working directory. Defaults to the current directory.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Initial task. Omit to start an interactive agent with no first prompt.
    #[arg(long)]
    pub prompt: Option<String>,
    /// Window/session name. Session placement derives it from cwd when omitted.
    #[arg(long)]
    pub name: Option<String>,
    /// Managed workspace/project. Valid only together with --work.
    #[arg(long)]
    pub workspace: Option<String>,
    /// Managed Work. Reuses its current Run window or creates it once.
    #[arg(long)]
    pub work: Option<String>,
    /// Optional agent role stored on the pane, for example reviewer.
    #[arg(long)]
    pub role: Option<String>,
    /// Optional short task label stored on the pane.
    #[arg(long)]
    pub task: Option<String>,
    /// Stable per-work name for this pane, used by `tend work up` to tell
    /// an agent it already started from one it still has to.
    #[arg(long)]
    pub alias: Option<String>,
    /// Provider argument for this launch, repeatable — for example
    /// `--option --model --option gpt-5-codex`. Replaces whatever
    /// `[agent.<program>]` configures for this provider rather than adding
    /// to it, so a repeated flag never reaches the CLI twice.
    #[arg(long = "option")]
    pub options: Vec<String>,
    /// Split to the right (the tmux default) or below the target pane.
    ///
    /// Optional rather than defaulting to `right`: a default the caller
    /// never typed used to trip the native host's "--direction requires
    /// tmux" check, so a plain `tend agent start` outside tmux always failed.
    #[arg(long, value_enum)]
    pub direction: Option<SplitDirection>,
    /// Emit the structured result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkStartArgs {
    /// Work id. Its current Run reuses or creates one managed tmux window.
    pub work: String,
    /// Workspace/project session. Defaults to the work directory name.
    #[arg(long)]
    pub workspace: Option<String>,
    /// First allowlisted agent, or another agent to add when the work exists.
    #[arg(long, value_enum)]
    pub agent: AgentProgram,
    /// Work directory. Defaults to current directory; checked when the work window is reused.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Initial task.
    #[arg(long)]
    pub prompt: Option<String>,
    /// Optional agent role, for example implementer or reviewer.
    #[arg(long)]
    pub role: Option<String>,
    /// Optional short task label.
    #[arg(long)]
    pub task: Option<String>,
    /// Stable per-work name for this pane, used by `tend work up`.
    #[arg(long)]
    pub alias: Option<String>,
    /// Provider argument for this launch, repeatable. Same rule as
    /// `tend agent start --option`: it replaces the configured defaults.
    #[arg(long = "option")]
    pub options: Vec<String>,
    /// Split a reused work window to the right or below.
    #[arg(long, value_enum, default_value = "right")]
    pub direction: SplitDirection,
    /// Emit the structured result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone)]
pub struct StartRequest {
    pub creator: Option<tend::collaboration::CollaborationEndpoint>,
    pub worker_spawn: Option<tend::collaboration::WorkerSpawnTicket>,
    pub bypass_permissions: bool,
    pub purpose: tend::collaboration::LaunchPurpose,
    pub agent: AgentProgram,
    /// Additional provider CLI arguments after Tend's built-in profile and
    /// before the optional initial prompt.
    pub options: Vec<String>,
    pub placement: Placement,
    pub target: Option<String>,
    pub cwd: Option<PathBuf>,
    pub prompt: Option<String>,
    pub name: Option<String>,
    pub workspace: Option<String>,
    pub work: Option<String>,
    pub role: Option<String>,
    pub task: Option<String>,
    pub alias: Option<String>,
    /// Durable pipeline generation stamped on an aliased pane. A later
    /// `work done` reads it back so an old pane cannot complete a new run.
    pub generation: Option<u64>,
    pub direction: SplitDirection,
    /// The daemon this launch belongs to.
    ///
    /// Reserving an alias used to go to `paths::default_socket()` regardless,
    /// so `--alias` against any other daemon reserved the name in the wrong
    /// room — or, with no daemon on the default socket at all, failed and
    /// rolled the whole launch back.
    pub socket: PathBuf,
    /// `[agent.<program>].account`: which account the agent starts
    /// on. `None` runs the bare provider CLI.
    pub account: Option<tend::swap_api::AccountRoute>,
}

/// The caller's own `--option`/`options` list, or `None` when they named
/// none. Empty and absent mean the same thing here — a caller cannot ask
/// for "no arguments at all" while a provider default exists, which is
/// what `[agent.<program>].options = []` is for.
fn explicit_options(options: &[String]) -> Option<&[String]> {
    (!options.is_empty()).then_some(options)
}

impl StartRequest {
    /// The request a `tend agent start` invocation describes, against the
    /// daemon the CLI resolved.
    ///
    /// `cfg` supplies the provider's configured launch arguments when the
    /// caller named none, so the CLI starts an agent the same way a
    /// pipeline or an MCP call does.
    pub fn from_args(args: &StartArgs, socket: &Path, cfg: &tend::config::Config) -> Self {
        Self {
            creator: None,
            worker_spawn: None,
            bypass_permissions: args.purpose != tend::collaboration::LaunchPurpose::Worker
                || cfg.collaboration.worker_bypass_permissions,
            purpose: args.purpose,
            agent: args.agent,
            options: cfg.launch_options(args.agent.label(), explicit_options(&args.options)),
            placement: args.placement,
            target: args.target.clone(),
            cwd: args.cwd.clone(),
            prompt: args.prompt.clone(),
            name: args.name.clone(),
            workspace: args.workspace.clone(),
            work: args.work.clone(),
            role: args.role.clone(),
            task: args.task.clone(),
            alias: args.alias.clone(),
            generation: None,
            direction: args.direction.unwrap_or(SplitDirection::Right),
            socket: socket.to_path_buf(),
            account: cfg.account_route(args.agent.label()),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StartResult {
    /// Verified composer state; process creation alone is not readiness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_readiness: Option<tend::worker_startup::Report>,
    pub launch_id: Option<String>,
    pub launch_receipt: Option<Box<tend::launch_identity::LaunchReceipt>>,
    pub host: LaunchHost,
    pub pane: String,
    pub agent: AgentProgram,
    pub placement: Placement,
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work: Option<String>,
    pub created_workspace: bool,
    pub created_work: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub cwd: PathBuf,
    pub prompt_supplied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<tend::swap_api::AccountDecision>,
}

/// Stable `tend agent start --json` envelope across execution hosts.
///
/// Host-specific coordinates remain nullable so consumers can branch on `host`.
/// Additive launch provenance is omitted when unavailable. `StartResult` remains
/// the tmux lifecycle result used by managed Work/MCP internals.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct AgentStartOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    input_readiness: Option<tend::worker_startup::Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_receipt: Option<Box<tend::launch_identity::LaunchReceipt>>,
    schema_version: u8,
    host: LaunchHost,
    agent: AgentProgram,
    placement: Placement,
    pane: Option<String>,
    session: Option<String>,
    window: Option<String>,
    name: Option<String>,
    workspace: Option<String>,
    work: Option<String>,
    created_workspace: bool,
    created_work: bool,
    role: Option<String>,
    task: Option<String>,
    alias: Option<String>,
    cwd: PathBuf,
    prompt_supplied: bool,
    account: Option<tend::swap_api::AccountDecision>,
}

impl AgentStartOutput {
    /// The line `tend agent start` prints without `--json`.
    fn describe(&self) -> String {
        let account = account_suffix(self.account.as_ref());
        let agent = self.agent.label();
        let cwd = self.cwd.display();
        match (self.host, self.session.as_deref(), self.pane.as_deref()) {
            (LaunchHost::Native, Some(session), _) => format!(
                "started {agent}{account} in native session {session} (cwd {cwd}){}\nattach with: tend proc attach {session}",
                self.not_ready_note()
            ),
            (_, _, pane) => format!(
                "started {agent}{account} in {} {} (cwd {cwd}){}",
                match self.placement {
                    Placement::Pane => "pane",
                    Placement::Window => "window pane",
                    Placement::Session => "session pane",
                },
                pane.unwrap_or_default(),
                self.not_ready_note(),
            ),
        }
    }

    /// `; not seen ready yet: …` for a start whose agent was not seen ready.
    fn not_ready_note(&self) -> String {
        self.input_readiness
            .as_ref()
            .filter(|report| report.state != tend::worker_startup::State::Ready)
            .map_or_else(String::new, |report| {
                format!(
                    "; not seen ready yet: {}",
                    report.reason.as_deref().unwrap_or("startup did not finish")
                )
            })
    }

    fn tmux(result: &StartResult) -> Self {
        Self {
            input_readiness: result.input_readiness.clone(),
            schema_version: 1,
            launch_id: result.launch_id.clone(),
            launch_receipt: result.launch_receipt.clone(),
            host: result.host,
            agent: result.agent,
            placement: result.placement,
            pane: Some(result.pane.clone()),
            session: result.session.clone(),
            window: result.window.clone(),
            name: result.name.clone(),
            workspace: result.workspace.clone(),
            work: result.work.clone(),
            created_workspace: result.created_workspace,
            created_work: result.created_work,
            role: result.role.clone(),
            task: result.task.clone(),
            alias: result.alias.clone(),
            cwd: result.cwd.clone(),
            prompt_supplied: result.prompt_supplied,
            account: result.account.clone(),
        }
    }
}

#[derive(clap::Parser)]
#[command(name = "tend agent start", no_binary_name = true)]
struct StartCommand {
    #[command(flatten)]
    args: StartArgs,
}

/// Parse the `["agent", "start", …]` argv tendd would hand the CLI.
pub fn parse_start_argv(argv: &[String]) -> Result<StartArgs> {
    let [command, subcommand, rest @ ..] = argv else {
        bail!("expected `agent start …`, got {argv:?}");
    };
    if command != "agent" || subcommand != "start" {
        bail!("expected `agent start …`, got {argv:?}");
    }
    <StartCommand as clap::Parser>::try_parse_from(rest)
        .map(|command| command.args)
        .map_err(|error| anyhow::anyhow!("{}", error.render()))
}

pub async fn run(
    args: StartArgs,
    client: &Client,
    socket_path: &Path,
    cfg: &tend::config::Config,
) -> Result<()> {
    let json = args.json;
    let env = crate::caller::caller_env(socket_path);
    let result = match launch(args, client, socket_path, cfg, env).await {
        Ok(result) => result,
        Err(error) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&failure_json(&error))?);
            }
            return Err(error);
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", result.describe());
    }
    Ok(())
}

/// `tend agent start --json` without printing: the envelope the CLI would
/// write to stdout. tendd calls this in-process. `native_env` is the
/// environment a native session gets, which the CLI takes from its own
/// (`caller::caller_env`). Synchronous tmux work runs on the calling
/// thread, so an async caller runs this on a blocking thread.
pub async fn launch_json(
    args: StartArgs,
    client: &Client,
    socket_path: &Path,
    cfg: &tend::config::Config,
    native_env: Vec<(String, String)>,
) -> Result<serde_json::Value> {
    let result = launch(args, client, socket_path, cfg, native_env).await?;
    Ok(serde_json::to_value(result)?)
}

async fn launch(
    args: StartArgs,
    client: &Client,
    socket_path: &Path,
    cfg: &tend::config::Config,
    native_env: Vec<(String, String)>,
) -> Result<AgentStartOutput> {
    match args.host.resolve(tend::backend::detect_host_env()) {
        LaunchHost::Native => {
            anyhow::ensure!(
                args.purpose == tend::collaboration::LaunchPurpose::Primary,
                "worker purpose currently requires a tmux/rmux agent pane"
            );
            launch_native(args, client, cfg, native_env).await
        }
        LaunchHost::Tmux => {
            let result = start(StartRequest::from_args(&args, socket_path, cfg))?;
            Ok(AgentStartOutput::tmux(&result))
        }
        LaunchHost::Auto => unreachable!("auto launch host is resolved above"),
    }
}

async fn launch_native(
    args: StartArgs,
    client: &Client,
    cfg: &tend::config::Config,
    env: Vec<(String, String)>,
) -> Result<AgentStartOutput> {
    if let Some(refusal) = native_refusal(&args) {
        bail!(refusal);
    }

    let cwd_source = args.cwd.unwrap_or(std::env::current_dir()?);
    let cwd = tend::paths::canonicalize(&cwd_source)
        .with_context(|| format!("resolve cwd {}", cwd_source.display()))?;
    if !cwd.is_dir() {
        bail!("cwd is not a directory: {}", cwd.display());
    }
    let prompt = args
        .prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty());
    // The native host resolves the provider's arguments the same way the
    // tmux one does: which surface a session lands on says nothing about
    // which model it should run.
    let options = cfg.launch_options(args.agent.label(), explicit_options(&args.options));
    let routed = route_account(cfg.account_route(args.agent.label()).as_ref(), &[])?;
    let launch = args
        .agent
        .native_launch(&options, prompt)
        .on_account(routed.as_ref());
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));
    // The one thing a native launch creates; not after an abandoned call.
    crate::subprocess::ensure_time_left()?;
    let session = client
        .spawn_session(tend::SpawnSession {
            command: launch.command,
            args: launch.args,
            env,
            cwd: Some(cwd.clone()),
            name: args
                .name
                .clone()
                .or_else(|| Some(args.agent.label().into())),
            cols: Some(cols),
            rows: Some(rows),
        })
        .await
        .context("spawning native tend agent session")?;
    let input_readiness = native_ready(
        client,
        args.agent,
        &session,
        prompt.is_some(),
        starts_fresh(args.agent, &options),
    )
    .await?;
    Ok(AgentStartOutput {
        input_readiness,
        launch_id: None,
        launch_receipt: None,
        schema_version: 1,
        host: LaunchHost::Native,
        agent: args.agent,
        placement: Placement::Session,
        pane: None,
        session: Some(session.id),
        window: None,
        name: session.display_name,
        workspace: None,
        work: None,
        created_workspace: false,
        created_work: false,
        role: None,
        task: None,
        alias: None,
        cwd,
        prompt_supplied: prompt.is_some(),
        account: routed.as_ref().map(tend::swap_api::report),
    })
}

/// Wait, within [`START_BUDGET`], for an agent just spawned in a native
/// session to be able to work, the way a tmux launch does: the screen
/// comes from `capture_session`, keys go in with `write_session_bytes`,
/// and the session is alive while it has not exited and still runs the
/// process it started with. `Err` ([`NotStarted`]) says why it did not
/// start; the session is kept. Never relaunched on another account: a
/// native session has no transcript id tend chose to prove the refusal.
async fn native_ready(
    client: &Client,
    agent: AgentProgram,
    session: &tend::session::SessionRef,
    prompted: bool,
    fresh: bool,
) -> Result<Option<tend::worker_startup::Report>> {
    use tend::worker_startup::State;
    let kind = match agent {
        AgentProgram::Claude => tend::event::AgentKind::ClaudeCode,
        AgentProgram::Codex => tend::event::AgentKind::Codex,
        AgentProgram::Antigravity | AgentProgram::Opencode => return Ok(None),
    };
    let client = client.clone();
    let id = session.id.clone();
    let pid = session.pid;
    let runtime = tokio::runtime::Handle::current();
    let timeout = START_BUDGET.saturating_sub(std::time::Duration::from_secs(4));
    let report = tokio::task::spawn_blocking(move || {
        let snapshot = || runtime.block_on(client.capture_session(&id)).ok();
        let live = |shot: &tend::session::TerminalSnapshot| {
            !shot.session.exited && (pid.is_none() || shot.session.pid == pid)
        };
        // `capture_session` hands back the raw output split at newlines,
        // escapes and cursor moves included; the startup checks need the
        // screen a terminal would show. Render it here from the session's
        // own bytes, read on from where the last read stopped.
        let size = snapshot().map_or((24, 80), |shot| (shot.rows, shot.cols));
        let screen = std::cell::RefCell::new((vt100::Parser::new(size.0, size.1, 0), 0_u64));
        let capture = || {
            snapshot().filter(|shot| live(shot))?;
            let mut screen = screen.borrow_mut();
            let (parser, offset) = &mut *screen;
            let out = runtime.block_on(client.read_session(&id, *offset)).ok()?;
            parser.process(&out.data_bytes().ok()?);
            *offset = out.next_offset;
            Some(parser.screen().contents())
        };
        let alive = || snapshot().is_some_and(|shot| live(&shot));
        let send = |key: &str| {
            alive()
                && runtime
                    .block_on(client.write_session_bytes(&id, key.as_bytes()))
                    .is_ok()
        };
        if prompted {
            tend::worker_startup::watch_prompted(kind, capture, alive, send, true, fresh, timeout)
        } else {
            tend::worker_startup::prepare(kind, capture, alive, send, true, fresh, timeout)
        }
    })
    .await
    .context("watch the native session start")?;
    let id = &session.id;
    let reason = report
        .reason
        .clone()
        .unwrap_or_else(|| "startup did not finish".into());
    match report.state {
        State::Ready | State::Unsettled => Ok(Some(report)),
        State::Exited => Err(anyhow::Error::new(NotStarted {
            message: format!("did not start: {reason}"),
            pane: None,
            session: Some(id.clone()),
            alias: None,
        })),
        state => Err(anyhow::Error::new(NotStarted {
            message: format!(
                "did not start: {reason}; native session {id} is kept{}",
                if state == State::Blocked {
                    " for you to answer"
                } else {
                    ""
                }
            ),
            pane: None,
            session: Some(id.clone()),
            alias: None,
        })),
    }
}

/// Why `args` cannot start on the native host, or `None` when they can.
fn native_refusal(args: &StartArgs) -> Option<&'static str> {
    if args.work.is_some() || args.workspace.is_some() {
        return Some(
            "managed --work/--workspace launch is not available on the native host yet; use --host tmux or `tend proc run`",
        );
    }
    if args.target.is_some() || args.placement != Placement::Pane {
        return Some(
            "--target and non-pane --placement require tmux; omit them for a native session or use --host tmux",
        );
    }
    if args
        .direction
        .is_some_and(|direction| direction != SplitDirection::Auto)
    {
        return Some("--direction requires tmux; omit it for a native session or use --host tmux");
    }
    if args.role.is_some() || args.task.is_some() || args.alias.is_some() {
        return Some(
            "--role, --task, and --alias require a managed Work binding, which is not available on the native host yet",
        );
    }
    None
}

pub fn run_work_start(
    args: WorkStartArgs,
    socket: &Path,
    cfg: &tend::config::Config,
) -> Result<()> {
    let json = args.json;
    let result = start(StartRequest {
        creator: None,
        worker_spawn: None,
        bypass_permissions: true,
        purpose: tend::collaboration::LaunchPurpose::Primary,
        socket: socket.to_path_buf(),
        agent: args.agent,
        options: cfg.launch_options(args.agent.label(), explicit_options(&args.options)),
        placement: Placement::Pane,
        target: None,
        cwd: args.cwd,
        prompt: args.prompt,
        name: None,
        workspace: args.workspace,
        work: Some(args.work),
        role: args.role,
        task: args.task,
        alias: args.alias,
        generation: None,
        direction: args.direction,
        account: cfg.account_route(args.agent.label()),
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "{} agent {} in workspace {} work {} (session {}, window {}, cwd {})",
            if result.created_work {
                "created"
            } else {
                "added"
            },
            result.pane,
            result.workspace.as_deref().unwrap_or("-"),
            result.work.as_deref().unwrap_or("-"),
            result.session.as_deref().unwrap_or("-"),
            result.window.as_deref().unwrap_or("-"),
            result.cwd.display()
        );
    }
    Ok(())
}

/// A dedicated Claude worker is explicitly created to receive delegated work.
/// Set its inbound policy for this process only, independently of tool approval
/// permissions. An operator-supplied settings file/value retains full control.
fn configure_worker_inbound(
    agent: AgentProgram,
    purpose: tend::collaboration::LaunchPurpose,
    options: &mut Vec<String>,
) {
    if agent != AgentProgram::Claude || purpose != tend::collaboration::LaunchPurpose::Worker {
        return;
    }
    let explicit_settings = options
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg == "--settings" || arg.starts_with("--settings="));
    if !explicit_settings {
        options.splice(
            0..0,
            [
                "--settings".into(),
                r#"{"crossSessionInbound":"accept"}"#.into(),
            ],
        );
    }
}

/// Tend's own bootstrap/skill files live outside the project. Make only that
/// managed directory available to workers so reading AGENTS instructions does
/// not itself trigger an external-read approval on a fresh account.
fn configure_worker_bootstrap_context(
    agent: AgentProgram,
    purpose: tend::collaboration::LaunchPurpose,
    options: &mut Vec<String>,
    directory: Option<&Path>,
) {
    if purpose != tend::collaboration::LaunchPurpose::Worker || agent != AgentProgram::Claude {
        return;
    }
    let Some(directory) = directory.filter(|p| p.join("bootstrap.md").is_file()) else {
        return;
    };
    let Ok(directory) = tend::paths::canonicalize(directory) else {
        return;
    };
    let at = options
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(options.len());
    options.splice(
        at..at,
        [format!("--add-dir={}", directory.to_string_lossy())],
    );
}

/// A queued Work (`Q-…` id) is created only by the Work queue's own
/// launch, whose claims carry a generation; any other start into one is
/// refused until the queue has released it. Released is final and nothing
/// else creates such a Work, so the answer cannot go stale before the
/// tmux calls below.
fn refuse_leased_queue_work(request: &StartRequest) -> Result<()> {
    let Some(work) = request.work.as_deref() else {
        return Ok(());
    };
    if request.generation.is_some() || !tend::pipeline_run::is_queue_work_id(work) {
        return Ok(());
    }
    let response = tend::ipc::blocking_call(
        &request.socket,
        &serde_json::json!({"protocol": tend::event::PROTOCOL_VERSION, "kind": "pipeline_runs"}),
        std::time::Duration::from_secs(2),
    );
    let runs: Vec<tend::pipeline_run::PipelineRun> = response
        .and_then(|response| serde_json::from_value(response["pipeline_runs"].clone()).ok())
        .unwrap_or_default();
    queue_work_released(&runs, work, request.workspace.as_deref())
}

/// Whether a start into queued Work `work` may go ahead: only once its Run
/// exists and its lease is released.
pub fn queue_work_released(
    runs: &[tend::pipeline_run::PipelineRun],
    work: &str,
    workspace: Option<&str>,
) -> Result<()> {
    let released = runs.iter().any(|run| {
        run.identity.work_id.eq_ignore_ascii_case(work)
            && workspace.is_none_or(|w| run.identity.workspace_id.eq_ignore_ascii_case(w))
            && !run.leased_against(None)
    });
    anyhow::ensure!(
        released,
        "{}: Work {work} is a queued Work the Work queue has not released; settle it in the queue first",
        tend::pipeline_run::WORK_RESERVED_PREFIX
    );
    Ok(())
}

/// An agent tend launched that did not start, as a typed error: callers
/// that report it (`work up`, MCP, `--json`) carry the kept pane as a field
/// rather than only in the sentence.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NotStarted {
    /// The whole sentence, as the error displays.
    pub message: String,
    /// The pane left for the person, when one was kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// The native session left for the person, when one was kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The pipeline alias it was launched for, when `work up` knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl std::fmt::Display for NotStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NotStarted {}

fn not_started(message: String, pane: Option<&str>) -> anyhow::Error {
    anyhow::Error::new(NotStarted {
        message,
        pane: pane.map(str::to_owned),
        session: None,
        alias: None,
    })
}

/// The [`NotStarted`] behind `error`, if a launch failed that way.
#[must_use]
pub fn not_started_in(error: &anyhow::Error) -> Option<&NotStarted> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<NotStarted>())
}

/// `error`'s JSON for a caller that reads fields: the message, and the
/// launch that did not start (its kept pane, its alias) when that is why.
#[must_use]
pub fn failure_json(error: &anyhow::Error) -> serde_json::Value {
    let mut value = serde_json::json!({ "ok": false, "error": format!("{error:#}") });
    if let Some(not_started) = not_started_in(error) {
        value["not_started"] = serde_json::json!(not_started);
    }
    value
}

/// How many accounts one launch tries when `auto` routing lands on one
/// that turns out to be at its limit.
const LIMIT_RELAUNCHES: usize = 2;

/// How long one start may spend, readiness waits and an account change
/// included. The app's and Fleet's `agent start` child is killed after 30
/// seconds (`WorkCommandLimits::COMMAND`), which must find it finished.
const START_BUDGET: std::time::Duration = std::time::Duration::from_secs(25);

/// The least time a second attempt on another account needs: a launch and
/// its readiness watch.
const RELAUNCH_FLOOR: std::time::Duration = std::time::Duration::from_secs(10);

/// Start one allowlisted agent in a detached tmux surface and return its exact
/// pane id. The operation is synchronous and should be wrapped in
/// `spawn_blocking` by async callers.
///
/// It waits, within a bound, until the agent can work: its startup
/// dialogs answered where tend may, its first prompt taken. One that cannot
/// is not left silent: the error says why it did not start (its pane is
/// kept for the person), and an `auto`-routed launch that meets the usage
/// limit starts again on another account instead, once that account is
/// chosen and the capped pane is confirmed closed.
pub fn start(request: StartRequest) -> Result<StartResult> {
    let deadline = std::time::Instant::now() + START_BUDGET;
    // For the row note of whichever attempt starts: its row is newer.
    let begun = LaunchFacts {
        deadline,
        launched_at: time::OffsetDateTime::now_utc(),
        session: None,
        prompt: None,
        roots: Vec::new(),
    };
    let auto = request
        .account
        .as_ref()
        .is_some_and(|route| route.choice == tend::swap_api::SpawnAccount::Auto);
    // A worker's spawn ticket admits exactly one physical create.
    let may_relaunch = auto && request.worker_spawn.is_none();
    let mut passed_over: Vec<String> = Vec::new();
    // Before anything is created: a slow or failed query, or a refusal,
    // never leaves half-built layout behind.
    let mut routed = route_account(request.account.as_ref(), &passed_over)?;
    loop {
        match start_once(request.clone(), routed.clone(), deadline)? {
            Attempt::Started(result) => {
                let mut result = *result;
                if let (Some(decision), false) = (result.account.as_mut(), passed_over.is_empty()) {
                    decision.passed_over.clone_from(&passed_over);
                    if let Some(account) = &decision.account {
                        note_on_row(
                            &request,
                            &result.pane,
                            result.launch_receipt.as_deref(),
                            &begun,
                            tend::event::NotificationLevel::Info,
                            &format!(
                                "started on {account} because {} was at its usage limit",
                                passed_over.join(", ")
                            ),
                        );
                    }
                }
                return Ok(result);
            }
            Attempt::Capped(capped) => {
                let kept = if !may_relaunch || passed_over.len() >= LIMIT_RELAUNCHES {
                    String::new()
                } else if let Some(why) = &capped.unproven {
                    format!("; not started on another account: {why}")
                } else {
                    match next_account(&request, &passed_over, &capped, deadline) {
                        Ok(next) => {
                            passed_over.push(capped.account.clone());
                            routed = Some(Ok(next));
                            continue;
                        }
                        Err(why) => why,
                    }
                };
                let message = format!("did not start: {} {}", capped.account, capped.reason);
                note_on_row(
                    &request,
                    &capped.pane,
                    Some(&capped.receipt),
                    &capped.facts,
                    tend::event::NotificationLevel::Error,
                    &message,
                );
                return Err(not_started(
                    format!("{message}{kept}; pane {} is kept", capped.pane),
                    Some(&capped.pane),
                ));
            }
        }
    }
}

/// Another account for a launch that came up capped on `capped.account`,
/// and the capped pane closed to make way for it. An account is chosen
/// first; only then is the pane closed, and only while it still holds this
/// launch, and the close must be confirmed. `Err` says why the capped pane
/// stays instead.
fn next_account(
    request: &StartRequest,
    passed_over: &[String],
    capped: &Capped,
    deadline: std::time::Instant,
) -> std::result::Result<tend::swap_api::RoutedLaunch, String> {
    let room = || deadline.saturating_duration_since(std::time::Instant::now()) >= RELAUNCH_FLOOR;
    if !room() {
        return Err("; not started on another account: no time left for one".into());
    }
    let route = request.account.as_ref().ok_or_else(String::new)?;
    let mut skip = passed_over.to_vec();
    skip.push(capped.account.clone());
    let next = match crate::account_module::decide(route, &skip) {
        Ok(next) if next.account != capped.account => next,
        Ok(_) => return Err("; no other account to start on".into()),
        Err(
            tend::swap_api::NotRouted::Capped(reason) | tend::swap_api::NotRouted::Bare(reason),
        ) => return Err(format!("; not started on another account: {reason}")),
    };
    // Choosing can take seconds (an `aas` query, Codex hook-trust prep).
    if !room() {
        return Err("; not started on another account: no time left for one".into());
    }
    release_capped_pane(capped)
        .map_err(|error| format!("; not started on another account: {error:#}"))?;
    Ok(next)
}

/// Close the pane of a launch that met the usage limit, re-proving at this
/// moment what made it safe: the pane still carries this launch and runs
/// the same process, and the conversation's own transcript still holds
/// only the one prompt the provider refused. Then confirm it is gone.
fn release_capped_pane(capped: &Capped) -> Result<()> {
    let pane = capped.pane.as_str();
    let control = crate::mux_control::Control::Ambient;
    let shown = control.output(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{@tend_launch_id}\t#{pane_pid}",
    ])?;
    let (launch, pid) = shown.trim_end().split_once('\t').unwrap_or_default();
    let process = pid
        .parse()
        .ok()
        .and_then(tend::collaboration::ProcessFence::capture);
    anyhow::ensure!(
        launch == capped.receipt.launch_id && process.is_some() && process == capped.process,
        "pane {pane} no longer runs this launch"
    );
    anyhow::ensure!(
        first_turn(&capped.facts) == tend::agent_conversation::FirstTurn::RejectedForLimit,
        "the agent in pane {pane} has done more than meet the limit"
    );
    // Nothing typed since: the pane still shows the refusal at an empty
    // composer. A draft (or a screen tend cannot read) keeps the pane.
    let screen = control
        .output(&["capture-pane", "-p", "-J", "-t", pane])
        .map(|raw| tend::screen::prepare_capture(&raw, 120))
        .unwrap_or_default();
    anyhow::ensure!(
        capped_screen_untouched(&screen),
        "pane {pane} no longer shows only the refusal (something was typed, or it could not be read)"
    );
    anyhow::ensure!(
        capped
            .facts
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            >= RELAUNCH_FLOOR,
        "no time left to start it on another account"
    );
    control
        .run(&["kill-pane", "-t", pane])
        .with_context(|| format!("could not close pane {pane}"))?;
    let gone = look_for(
        std::time::Duration::from_secs(1),
        std::time::Duration::from_millis(100),
        || pane_gone(pane).then_some(()),
    );
    anyhow::ensure!(gone.is_some(), "pane {pane} is not confirmed closed");
    Ok(())
}

/// What Claude shows in an empty composer after an error, read as text.
const CLAUDE_ERROR_PLACEHOLDERS: &[&str] = &["Once fixed, resend your message"];

/// Whether a capped Claude pane still shows only its refusal: the limit
/// line, no turn running, and Claude's composer frame recognised with
/// nothing typed in it. Anything tend cannot positively read as that
/// (a clipped frame, an unframed line) is not.
fn capped_screen_untouched(screen: &str) -> bool {
    if !matches!(
        tend::agent_tui::launch_blocker(screen),
        Some(tend::agent_tui::LaunchBlocker::Limit(_))
    ) || tend::agent_tui::turn_running(screen)
    {
        return false;
    }
    let lines: Vec<&str> = screen.lines().collect();
    let rule = |line: &&str| {
        let line = line.trim();
        line.chars().count() >= 5 && line.chars().all(|c| c == '─')
    };
    let rules: Vec<usize> = (0..lines.len()).filter(|at| rule(&lines[*at])).collect();
    let [.., top, bottom] = rules[..] else {
        return false;
    };
    let inside: Vec<&str> = lines[top + 1..bottom]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    let [only] = inside[..] else {
        return false;
    };
    // The frame must be the current composer: under it only Claude's few
    // footer lines (mode, status line), never another prompt.
    let footer: Vec<&str> = lines[bottom + 1..]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    if footer.len() > 4
        || footer
            .iter()
            .any(|line| line.starts_with(['❯', '›', '>', '$', '#', '%']) || rule(line))
    {
        return false;
    }
    only.strip_prefix('❯').is_some_and(|rest| {
        let rest = rest.trim_matches(|c: char| c.is_whitespace() || c == '\u{a0}');
        rest.is_empty() || CLAUDE_ERROR_PLACEHOLDERS.contains(&rest)
    })
}

/// Whether the server says `pane` does not exist (or the server itself is
/// gone). Any other failure to ask is not proof of absence.
fn pane_gone(pane: &str) -> bool {
    match crate::mux_control::Control::Ambient.output(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{pane_id}",
    ]) {
        Ok(shown) => shown.trim() != pane,
        Err(error) => {
            let error = format!("{error:#}").to_lowercase();
            ["can't find pane", "no such pane", "no server running"]
                .iter()
                .any(|absent| error.contains(absent))
        }
    }
}

/// What one launch knows about itself, for its later checks.
#[derive(Debug, Clone)]
struct LaunchFacts {
    /// When the start's time is up.
    deadline: std::time::Instant,
    /// Just before the pane was created: a row older than this is not
    /// this launch's.
    launched_at: time::OffsetDateTime,
    /// The Claude conversation id tend itself generated for this launch
    /// (never one the caller chose).
    session: Option<String>,
    /// The first prompt tend passed on the command line.
    prompt: Option<String>,
    /// Where that conversation's transcript may be.
    roots: Vec<PathBuf>,
}

/// What the launch's own transcript says its first turn did.
fn first_turn(facts: &LaunchFacts) -> tend::agent_conversation::FirstTurn {
    facts
        .session
        .as_deref()
        .zip(facts.prompt.as_deref())
        .map_or(
            tend::agent_conversation::FirstTurn::Unknown,
            |(session, prompt)| {
                tend::agent_conversation::claude_first_turn(
                    session,
                    &facts.roots,
                    prompt,
                    std::time::SystemTime::from(facts.launched_at),
                )
            },
        )
}

/// Where a Claude launch on `routed` keeps its transcripts: the account's
/// own `CLAUDE_CONFIG_DIR` when the launch sets one, then the usual places.
fn claude_roots_for(
    routed: Option<&std::result::Result<tend::swap_api::RoutedLaunch, String>>,
    pane_pid: u32,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = routed
        .and_then(|routed| routed.as_ref().ok())
        .into_iter()
        .flat_map(|routed| routed.launcher.iter())
        .filter_map(|word| word.strip_prefix("CLAUDE_CONFIG_DIR="))
        .map(PathBuf::from)
        .collect();
    roots.extend(tend::agent_conversation::claude_roots(pane_pid));
    roots
}

/// One physical launch.
enum Attempt {
    Started(Box<StartResult>),
    /// The agent came up at its account's usage limit.
    Capped(Box<Capped>),
}

/// A launch that met its account's usage limit.
struct Capped {
    account: String,
    pane: String,
    /// The process tmux started for this launch, start time included.
    process: Option<tend::collaboration::ProcessFence>,
    receipt: tend::launch_identity::LaunchReceipt,
    facts: LaunchFacts,
    reason: String,
    /// Why it may not be closed and started again elsewhere: the screen
    /// alone is not proof that the provider refused the prompt before any
    /// work. `None`: the transcript proves it.
    unproven: Option<String>,
}

#[allow(clippy::too_many_lines)] // launch, metadata stamping, and rollback form one physical transaction
fn start_once(
    mut request: StartRequest,
    routed: Option<std::result::Result<tend::swap_api::RoutedLaunch, String>>,
    deadline: std::time::Instant,
) -> Result<Attempt> {
    refuse_leased_queue_work(&request)?;
    configure_worker_inbound(request.agent, request.purpose, &mut request.options);
    let bootstrap = tend::paths::data_dir().map(|root| root.join("tend/agent-integration"));
    configure_worker_bootstrap_context(
        request.agent,
        request.purpose,
        &mut request.options,
        bootstrap.as_deref(),
    );
    let launch_id = request.worker_spawn.as_ref().map_or_else(
        || uuid::Uuid::new_v4().to_string(),
        |ticket| ticket.launch_id.clone(),
    );
    if request.purpose == tend::collaboration::LaunchPurpose::Worker {
        // Dedicated workers must report their own pane, rather than inherit
        // a human TUI's environment through Codex's shared app-server.
        if request.agent == AgentProgram::Codex
            && !request.options.iter().any(|arg| arg == "--no-daemon")
        {
            request.options.push("--no-daemon".into());
        }
        let hello = tend::ipc::blocking_call(&request.socket,
            &serde_json::json!({"protocol":tend::event::PROTOCOL_VERSION,"kind":"hello","client":"tend-cli"}),
            std::time::Duration::from_secs(2)).context("worker purpose capability check failed")?;
        anyhow::ensure!(
            hello["capabilities"].as_array().is_some_and(|caps| caps
                .iter()
                .any(|cap| cap == tend::collaboration::HUMAN_PRIORITY_CAPABILITY)),
            "worker launch requires an upgraded tendd with human-priority support"
        );
    }
    if request.creator.is_none() {
        request.creator = crate::spawn_provenance::resolve_creator(&request.socket);
    }
    let archive = tend::launch_identity::archive_directory();
    let options_given = request.options.len();
    let expected_session = if request.agent == AgentProgram::Claude {
        claude_session_options(&mut request.options)
    } else {
        None
    };
    // Tend generated the id (rather than taking the caller's) only when it
    // appended `--session-id <id>` itself.
    let session_generated = request.options.len() == options_given + 2;
    let archive = if let Some(directory) = archive {
        match tend::launch_identity::prepare(
            &directory,
            &tend::launch_identity::LaunchRecord {
                launch_id: launch_id.clone(),
                provider: tend::discovery::classify_command(request.agent.label()),
                expected_session: expected_session.clone(),
                wrapper_forks: routed.as_ref().is_some_and(std::result::Result::is_ok),
                no_exec: cfg!(windows),
                ..Default::default()
            },
        ) {
            Ok(()) => Some(directory),
            // An existing attempt must still never create a second physical process.
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && directory.join(format!("{launch_id}.json")).exists() =>
            {
                return Err(error)
                    .context("launch attempt already recorded; refusing duplicate create");
            }
            Err(error) => {
                eprintln!("warning: cannot prepare launch identity: {error}");
                None
            }
        }
    } else {
        eprintln!("warning: no launch archive directory; launch identity unavailable");
        None
    };
    let PreparedLaunch {
        cwd,
        workspace,
        work,
        created_workspace,
        created_work,
        adopted_session,
    } = prepare_launch(&mut request)?;
    let prompt = request
        .prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty());
    // Replace tmux's command shell, so the pane's foreground command and PID
    // describe the CLI even before it emits a hook. rmux on Windows hands the
    // line to PowerShell, which has no `exec`; there the program just runs.
    let command = tend::shell::Dialect::native()
        .exec_line_with_env(
            &request
                .agent
                .permission_launch(&request.options, prompt, request.bypass_permissions)
                .on_account(routed.as_ref())
                .argv(),
            &[("TEND_LAUNCH_ID", &launch_id)],
        )
        .map_err(anyhow::Error::msg)?;
    let args = tmux_args(&request, &cwd, &command)?;
    // Routing (an `aas` query, Codex hook-trust prep) can take seconds:
    // never create what this start has no time left to watch.
    if deadline.saturating_duration_since(std::time::Instant::now()) < RELAUNCH_FLOOR / 2 {
        return Err(not_started(
            "did not start: no time was left to launch and watch it after choosing an account"
                .into(),
            None,
        ));
    }
    let launched_at = time::OffsetDateTime::now_utc();
    if let Some(ticket) = &request.worker_spawn {
        update_spawn(
            &request.socket,
            ticket,
            &tend::collaboration::WorkerSpawnAction::Begin,
        )?;
    }
    let output =
        crate::subprocess::output(&mut crate::mux_control::ambient_command_with_args(&args)?)
            .context("run tmux agent launcher")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if create_was_rejected(&output) {
            if let Some(directory) = &archive {
                let _ = tend::launch_identity::failed(directory, &launch_id);
            }
            if let Some(ticket) = &request.worker_spawn {
                // Only a completed, unsuccessful single create command with
                // no pane ID is definite. A missing reply or partial output
                // remains uncertain and is never silently retried.
                update_spawn(
                    &request.socket,
                    ticket,
                    &tend::collaboration::WorkerSpawnAction::CreateRejected {
                        detail: stderr.clone(),
                    },
                )?;
            }
        }
        // Name the resolved `-t` value: the input the caller gave (a pane id,
        // a session name) is not what tmux was asked for, and the difference
        // between them is usually the bug.
        let target = args
            .iter()
            .position(|arg| arg == "-t")
            .and_then(|at| args.get(at + 1))
            .map_or(String::new(), |target| format!(" (target {target:?})"));
        bail!(
            "tmux {} failed{target}{}",
            args.first().map_or("command", String::as_str),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    let rmux = crate::mux_control::ambient_rmux().is_some();
    let mut receipt = tend::launch_identity::LaunchReceipt::parse(
        &String::from_utf8_lossy(&output.stdout),
        &launch_id,
        None,
        rmux,
    )
    .context("tmux created the surface but returned no pane id; creation will not be retried")?;
    let pane = receipt
        .pane
        .strip_prefix("rmux:")
        .unwrap_or(&receipt.pane)
        .to_owned();
    // Query only the server that performed this create, never the daemon's ambient pane.
    receipt.endpoint = crate::mux_control::ambient_command_with_args(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "#{socket_path}",
    ])
    .ok()
    .and_then(|mut command| crate::subprocess::output(&mut command).ok())
    .filter(|out| out.status.success())
    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    .filter(|endpoint| !endpoint.is_empty() && !endpoint.contains("#{"));
    receipt.marked = crate::mux_control::ambient_command_with_args(&[
        "set-option",
        "-p",
        "-t",
        &pane,
        "@tend_launch_id",
        &launch_id,
    ])
    .ok()
    .and_then(|mut command| crate::subprocess::status(&mut command).ok())
    .is_some_and(|status| status.success());
    if let Some(directory) = &archive {
        if let Err(error) = tend::launch_identity::complete(directory, receipt.clone()) {
            // Provenance failure must not skip worker stamps or ordinary registration
            // after the physical create has already succeeded.
            eprintln!("warning: cannot archive launch receipt: {error}");
        }
    }

    if request.purpose == tend::collaboration::LaunchPurpose::Worker {
        // Launching was persisted before creation. A crash in the small
        // create-to-stamp gap remains Uncertain and cannot trigger a respawn.
        // Once stamped, any daemon client can reconcile this exact launch.
        crate::subprocess::output(&mut crate::mux_control::ambient_command_with_args(&[
            "set-option",
            "-p",
            "-t",
            &pane,
            "@tend_worker_launch",
            &launch_id,
        ])?)
        .and_then(|output| {
            if output.status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other("cannot stamp worker launch"))
            }
        })
        .context("record worker launch identity; pane left for reconciliation")?;
        if prompt.is_none() && matches!(request.agent, AgentProgram::Claude | AgentProgram::Codex) {
            crate::mux_control::Control::Ambient
                .run(&[
                    "set-option",
                    "-p",
                    "-t",
                    &pane,
                    "@tend_input_readiness",
                    "pending",
                ])
                .context("record worker preparation state")?;
        }
    }

    let managed = work.is_some();
    let session = managed
        .then(|| crate::tmux_work::session_name_for_pane(&pane))
        .transpose()?;
    let window = managed
        .then(|| crate::tmux_work::window_id_for_pane(&pane))
        .transpose()?;
    // The name is what the result reports to a human; the id is what the
    // marks are written against. They are read from the same pane, so they
    // describe one session — but only the id still says so once it reaches
    // tmux, which resolves a bare name by unique prefix as well as by
    // exact match.
    let session_id = managed
        .then(|| crate::tmux_work::session_id_for_pane(&pane))
        .transpose()?;

    let mark = (|| {
        if let (Some(session), Some(workspace)) = (&adopted_session, workspace.as_deref()) {
            crate::tmux_work::adopt_workspace(session, workspace)?;
        }
        if created_workspace {
            crate::tmux_work::mark_workspace(
                session_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("created workspace has no tmux session"))?,
                workspace
                    .as_deref()
                    .expect("created workspace has workspace id"),
                &cwd,
            )?;
        }
        if created_work {
            let window = window
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("created work has no tmux window id"))?;
            crate::tmux_work::mark_work(
                window,
                workspace.as_deref().expect("created work has workspace id"),
                work.as_deref().expect("created work has id"),
                &cwd,
            )?;
        }
        crate::tmux_work::mark_agent(
            &pane,
            request.agent.label(),
            workspace.as_deref(),
            work.as_deref(),
            request.role.as_deref(),
            request.task.as_deref(),
            request.alias.as_deref(),
            request.generation,
            &request.socket,
        )
    })();
    if let Err(error) = mark {
        if pane_gone(&pane) {
            return Err(not_started(
                format!("did not start: the agent exited as it started (its pane closed before tend could record it): {error:#}"),
                None,
            ));
        }
        crate::tmux_work::cleanup_pane(&pane);
        return Err(error).context("record tend tmux metadata");
    }
    if let Some(title) =
        crate::surface_names::agent_pane_title(request.alias.as_deref(), request.role.as_deref())
    {
        crate::surface_names::title_pane(&pane, &title);
    }
    // Hooks may not fire until the first prompt. Register the observed CLI
    // now, using discovery's placeholder so a later real hook replaces it.
    // Missing/older daemons remain supported; periodic discovery retries.
    register_launched_agent(&request, &pane);
    if request.purpose == tend::collaboration::LaunchPurpose::Worker {
        if let Err(error) = register_worker_purpose(&request, &pane, &launch_id) {
            crate::tmux_work::cleanup_pane(&pane);
            return Err(error).context("register worker launch purpose");
        }
    }
    crate::spawn_provenance::record(&request, &pane, &launch_id);
    let facts = LaunchFacts {
        deadline,
        launched_at,
        session: expected_session.filter(|_| session_generated),
        prompt: prompt.map(str::to_owned),
        roots: Vec::new(),
    };
    let input_readiness = match await_ready(
        &request,
        &pane,
        &cwd,
        prompt.is_some(),
        &receipt,
        routed.as_ref(),
        facts,
    )? {
        Ready::Checked(report) => report,
        Ready::Capped(capped) => return Ok(capped),
    };

    // Minting belongs to the agent's session-start hook, which reaches the
    // room's arbiter. Doing it here too would allocate from a namespace this
    // process cannot see all of — the bug the arbiter exists to close — so an
    // unaliased launch reports no handle and the hook names the pane a moment
    // later.
    let alias = request.alias;

    Ok(Attempt::Started(Box::new(StartResult {
        input_readiness,
        launch_id: Some(launch_id),
        launch_receipt: Some(Box::new(receipt)),
        host: LaunchHost::Tmux,
        pane,
        agent: request.agent,
        placement: request.placement,
        name: request.name,
        workspace,
        work,
        created_workspace,
        created_work,
        session,
        window,
        role: request.role,
        task: request.task,
        alias,
        cwd,
        prompt_supplied: prompt.is_some(),
        account: routed.as_ref().map(tend::swap_api::report),
    })))
}

/// What [`await_ready`] found.
enum Ready {
    /// Started: the readiness report, when the provider has a probe.
    Checked(Option<tend::worker_startup::Report>),
    /// Came up at its account's usage limit, on an account routing chose.
    Capped(Attempt),
}

/// How long one readiness watch may take: what the start's budget has
/// left, at most 30 seconds.
fn watch_budget(deadline: std::time::Instant) -> std::time::Duration {
    // Room after the watch for stamping the result and a row note.
    deadline
        .saturating_duration_since(std::time::Instant::now())
        .saturating_sub(std::time::Duration::from_secs(4))
        .min(std::time::Duration::from_secs(30))
}

/// Whether this launch starts a new conversation. Only a new one is read
/// for the usage-limit and sign-in screens: a resumed conversation's
/// history can show the same lines.
fn starts_fresh(agent: AgentProgram, options: &[String]) -> bool {
    let resumes: &[&str] = match agent {
        AgentProgram::Claude => &[
            "--resume",
            "-r",
            "--continue",
            "-c",
            "--fork-session",
            "--from-pr",
        ],
        AgentProgram::Codex => &["resume", "fork", "--last"],
        AgentProgram::Antigravity | AgentProgram::Opencode => &[],
    };
    !options.iter().any(|option| {
        let flag = option
            .split_once('=')
            .map_or(option.as_str(), |(flag, _)| flag);
        resumes.contains(&flag)
    })
}

/// Wait, within a bound, for the agent just launched in `pane` to be able
/// to work. `Err` says why it did not start; its pane is kept.
fn await_ready(
    request: &StartRequest,
    pane: &str,
    cwd: &Path,
    prompted: bool,
    receipt: &tend::launch_identity::LaunchReceipt,
    routed: Option<&std::result::Result<tend::swap_api::RoutedLaunch, String>>,
    facts: LaunchFacts,
) -> Result<Ready> {
    use tend::worker_startup::State;
    if !matches!(request.agent, AgentProgram::Claude | AgentProgram::Codex) {
        return Ok(Ready::Checked(None));
    }
    let launch_id = receipt.launch_id.as_str();
    // A worker launched without a prompt gets its task typed in later, and
    // only once its pane is proven ready: it stays strict.
    let worker = request.purpose == tend::collaboration::LaunchPurpose::Worker && !prompted;
    let watched = crate::worker_startup::watch_launch(
        pane,
        cwd,
        launch_id,
        request.bypass_permissions,
        prompted,
        starts_fresh(request.agent, &request.options),
        watch_budget(facts.deadline),
    );
    let report = match watched {
        Ok(report) => report,
        // A worker's input is only ever sent to a prepared pane.
        Err(error) if worker => return Err(error),
        // Gone before it could be watched: it exited.
        Err(_) if crate::mux_control::resolve_pane(pane).is_none() => {
            tend::worker_startup::Report {
                state: State::Exited,
                answered: Vec::new(),
                reason: Some("the agent exited during startup (its pane is gone)".into()),
            }
        }
        Err(error) => {
            eprintln!("tend: could not watch {pane} start: {error:#}");
            tend::worker_startup::Report {
                state: State::Unsettled,
                answered: Vec::new(),
                reason: Some(format!("{error:#}")),
            }
        }
    };
    let note = |facts: &LaunchFacts, level, message: &str| {
        note_on_row(request, pane, Some(receipt), facts, level, message);
    };
    let reason = report
        .reason
        .clone()
        .unwrap_or_else(|| "startup did not finish".into());
    match report.state {
        State::Ready => {}
        State::Limited => {
            if let Some(Ok(routed)) = routed {
                return Ok(Ready::Capped(Attempt::Capped(Box::new(capped_launch(
                    request, pane, receipt, routed, facts, &reason,
                )))));
            }
            let message = format!("did not start: {reason}");
            note(&facts, tend::event::NotificationLevel::Error, &message);
            return Err(not_started(
                format!("{message}; pane {pane} is kept"),
                Some(pane),
            ));
        }
        // Slow, not stuck: no dialog or blocker was seen. It counts as
        // started, and says so.
        State::Unsettled if !worker => {
            eprintln!("tend: {pane} started but is not seen ready yet: {reason}");
            note(
                &facts,
                tend::event::NotificationLevel::Warning,
                &format!("started, not seen ready yet: {reason}"),
            );
        }
        state => {
            let blocked = state == State::Blocked;
            note(
                &facts,
                if blocked {
                    tend::event::NotificationLevel::NeedsInput
                } else {
                    tend::event::NotificationLevel::Error
                },
                &format!("did not start: {reason}"),
            );
            if worker {
                return Err(not_started(
                    format!("worker pane {pane}, launch {launch_id}, was created but is not input-ready ({state:?}): {reason}; the pane is retained, no task was sent"),
                    Some(pane),
                ));
            }
            if state == State::Exited {
                return Err(not_started(format!("did not start: {reason}"), None));
            }
            return Err(not_started(
                format!(
                    "did not start: {reason}; pane {pane} is kept{}",
                    if blocked { " for you to answer" } else { "" }
                ),
                Some(pane),
            ));
        }
    }
    Ok(Ready::Checked(Some(report)))
}

/// A launch that met the limit on an account routing chose, with whether
/// its own transcript proves the provider refused the prompt before any
/// work (the only case it may be closed and started again elsewhere).
fn capped_launch(
    request: &StartRequest,
    pane: &str,
    receipt: &tend::launch_identity::LaunchReceipt,
    routed: &tend::swap_api::RoutedLaunch,
    mut facts: LaunchFacts,
    reason: &str,
) -> Capped {
    let pane_pid = pane_pid(pane);
    facts.roots = claude_roots_for(Some(&Ok(routed.clone())), pane_pid.unwrap_or(0));
    let process = pane_pid.and_then(tend::collaboration::ProcessFence::capture);
    let unproven = match (request.agent, &process) {
        (AgentProgram::Claude, _) if facts.prompt.is_none() => {
            Some("tend gave it no prompt, so a refused one is not tend's to replay".into())
        }
        (AgentProgram::Claude, _) if facts.session.is_none() => {
            Some("its conversation id was not one tend generated".into())
        }
        (AgentProgram::Claude, Some(_)) => {
            // The transcript can trail the screen by a moment.
            let proven = look_for(
                facts
                    .deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .saturating_sub(std::time::Duration::from_secs(4))
                    .min(std::time::Duration::from_secs(2)),
                std::time::Duration::from_millis(200),
                || {
                    (first_turn(&facts) == tend::agent_conversation::FirstTurn::RejectedForLimit)
                        .then_some(())
                },
            );
            proven.is_none().then(|| {
                "the agent's own transcript does not show the prompt refused before any work"
                    .to_owned()
            })
        }
        (AgentProgram::Claude, None) => Some("its process could not be read".into()),
        _ => Some("only a Claude launch's transcript can prove it did no work".into()),
    };
    Capped {
        account: routed.account.clone(),
        pane: pane.to_owned(),
        process,
        receipt: receipt.clone(),
        facts,
        reason: reason.replacen("the account is", "is", 1),
        unproven,
    }
}

/// The process tmux started in `pane`.
fn pane_pid(pane: &str) -> Option<u32> {
    crate::mux_control::Control::Ambient
        .output(&["display-message", "-p", "-t", pane, "#{pane_pid}"])
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Put `message` on the launched agent's row, so the person sees why it
/// did not start (or where it did) without the launch's own output. Only
/// on a row that is unmistakably this launch's: the receipt's pane on the
/// receipt's server, of the launched provider, and the only such row. A
/// notification can change a row's state, so anything less is skipped, as
/// is a missing daemon.
fn note_on_row(
    request: &StartRequest,
    pane: &str,
    receipt: Option<&tend::launch_identity::LaunchReceipt>,
    facts: &LaunchFacts,
    level: tend::event::NotificationLevel,
    message: &str,
) {
    // Best effort, never past the start's deadline.
    let left = facts
        .deadline
        .saturating_duration_since(std::time::Instant::now())
        .saturating_sub(std::time::Duration::from_millis(500));
    if left < std::time::Duration::from_millis(500) {
        return;
    }
    let wait = left.min(std::time::Duration::from_secs(2)) / 2;
    let Some(endpoint) = receipt.and_then(|receipt| receipt.endpoint.as_deref()) else {
        return;
    };
    let pane = receipt.map_or(pane, |receipt| receipt.pane.as_str());
    let call = |body: serde_json::Value| tend::ipc::blocking_call(&request.socket, &body, wait);
    let Some(reply) = call(serde_json::json!({
        "protocol": tend::event::PROTOCOL_VERSION,
        "kind": "by_pane",
        "pane": pane,
    })) else {
        return;
    };
    let rows: Vec<tend::state::Agent> =
        serde_json::from_value(reply["agents"].clone()).unwrap_or_default();
    let wanted = tend::discovery::classify_command(request.agent.label());
    let server = |socket: &str| {
        let path = tend::tmux::socket_path_or_default(socket);
        std::fs::canonicalize(&path).unwrap_or(path)
    };
    // A row this launch could have made: same provider, started since the
    // pane was created (an older one on a reused pane id is not it).
    let mut ours = rows.into_iter().filter(|row| {
        Some(row.kind) == wanted
            && row.started_at >= facts.launched_at - time::Duration::seconds(1)
            && row
                .tmux_socket
                .as_deref()
                .is_some_and(|socket| server(socket) == server(endpoint))
    });
    let (Some(row), None) = (ours.next(), ours.next()) else {
        return;
    };
    let event = tend::event::AgentEvent::NotificationFired {
        id: tend::event::AgentId {
            kind: row.kind,
            session_id: row.session_id,
            surface: row.surface,
            pane: row.pane,
            tmux_socket: row.tmux_socket,
            cwd: row.cwd,
        },
        level,
        message: message.to_owned(),
        at: time::OffsetDateTime::now_utc(),
    };
    let _ = call(serde_json::json!({
        "protocol": tend::event::PROTOCOL_VERSION,
        "kind": "ingest",
        "event": event,
    }));
}

/// A provider selection flag owns its own semantics; never inject a competing ID.
fn claude_session_options(options: &mut Vec<String>) -> Option<String> {
    let mut selection = false;
    let mut expected = None;
    for (index, arg) in options.iter().enumerate() {
        let (flag, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(flag, value)| (flag, Some(value)));
        if matches!(
            flag,
            "--resume" | "-r" | "--session-id" | "--continue" | "-c" | "--fork-session"
        ) {
            selection = true;
            if matches!(flag, "--resume" | "-r" | "--session-id") {
                expected = inline
                    .or_else(|| options.get(index + 1).map(String::as_str))
                    .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                    .map(str::to_owned);
            }
        }
    }
    if selection {
        return if options.iter().any(|arg| arg == "--fork-session") {
            None
        } else {
            expected
        };
    }
    let id = uuid::Uuid::new_v4().to_string();
    options.extend(["--session-id".into(), id.clone()]);
    Some(id)
}

/// ` on <account>` for a routed launch's one-line report.
fn account_suffix(decision: Option<&tend::swap_api::AccountDecision>) -> String {
    let Some((decision, account)) =
        decision.and_then(|decision| Some((decision, decision.account.as_deref()?)))
    else {
        return String::new();
    };
    if decision.passed_over.is_empty() {
        format!(" on {account}")
    } else {
        format!(
            " on {account} because {} was at its usage limit",
            decision.passed_over.join(", ")
        )
    }
}

fn create_was_rejected(output: &std::process::Output) -> bool {
    !output.status.success()
        && !String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim().starts_with('%'))
}

fn update_spawn(
    socket: &Path,
    ticket: &tend::collaboration::WorkerSpawnTicket,
    action: &tend::collaboration::WorkerSpawnAction,
) -> Result<()> {
    let response = tend::ipc::blocking_call(socket, &serde_json::json!({
        "protocol":tend::event::PROTOCOL_VERSION, "kind":"collaboration_worker_spawn", "ticket":ticket, "action":action,
    }), std::time::Duration::from_secs(5)).context("worker launch coordination timed out")?;
    anyhow::ensure!(
        response["ok"] == true,
        "{}",
        response["error"]
            .as_str()
            .unwrap_or("worker launch coordination failed")
    );
    Ok(())
}

fn register_worker_purpose(request: &StartRequest, pane: &str, launch_id: &str) -> Result<()> {
    let observed =
        crate::mux_control::resolve_pane(pane).context("worker pane is not observable")?;
    if let Some(ticket) = &request.worker_spawn {
        return update_spawn(
            &request.socket,
            ticket,
            &tend::collaboration::WorkerSpawnAction::Observe {
                pane: observed.pane_id,
                socket: observed.socket,
            },
        );
    }
    let kind = tend::discovery::classify_command(request.agent.label())
        .context("unknown worker provider")?;
    let result = tend::ipc::blocking_call(
        &request.socket,
        &serde_json::json!({
            "protocol":tend::event::PROTOCOL_VERSION,"kind":"collaboration_worker_launch",
            "pane":observed.pane_id,"socket":observed.socket,"agent_kind":kind,
            "launch_id":launch_id,
        }),
        std::time::Duration::from_secs(5),
    )
    .context("worker purpose registration timed out")?;
    anyhow::ensure!(
        result["ok"] == true,
        "{}",
        result["error"]
            .as_str()
            .unwrap_or("worker registration failed")
    );
    Ok(())
}

fn register_launched_agent(request: &StartRequest, pane: &str) {
    let Some(discovered) = launched_agent(request, pane, LAUNCH_SIGHTING) else {
        return;
    };
    let event = tend::discovery::synthesize_started(&discovered, time::OffsetDateTime::now_utc());
    let _ = tend::ipc::blocking_call(
        &request.socket,
        &serde_json::json!({
            "protocol": tend::event::PROTOCOL_VERSION,
            "kind": "ingest",
            "event": event,
        }),
        std::time::Duration::from_secs(2),
    );
}

/// How long a launch looks for its agent in the pane: the pane's shell may
/// not have started it yet when the window appears.
const LAUNCH_SIGHTING: std::time::Duration = std::time::Duration::from_secs(5);

/// The launched agent as discovery sees it in `pane`, once the pane runs
/// the requested CLI, looked for until `within` runs out. A read that sees
/// nothing (the pane not listed) is tried again too: a backend reports a
/// failed or timed-out listing the same way as a missing pane.
fn launched_agent(
    request: &StartRequest,
    pane: &str,
    within: std::time::Duration,
) -> Option<tend::discovery::Discovered> {
    let wanted = tend::discovery::classify_command(request.agent.label());
    look_for(within, std::time::Duration::from_millis(100), || {
        let observed = crate::mux_control::resolve_pane(pane)?;
        tend::discovery::discover_from_panes(&[observed])
            .into_iter()
            .next()
            .filter(|discovered| wanted == Some(discovered.kind))
    })
}

/// `look` until it finds something or `within` runs out, `every` apart.
fn look_for<T>(
    within: std::time::Duration,
    every: std::time::Duration,
    mut look: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = std::time::Instant::now() + within;
    loop {
        if let Some(found) = look() {
            return Some(found);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(every);
    }
}

struct PreparedLaunch {
    cwd: PathBuf,
    /// Session tend put this work into without having created it, as a tmux
    /// session id. It gets a workspace identity, never the managed flag.
    adopted_session: Option<String>,
    workspace: Option<String>,
    work: Option<String>,
    created_workspace: bool,
    created_work: bool,
}

fn prepare_launch(request: &mut StartRequest) -> Result<PreparedLaunch> {
    if request.workspace.is_some() && request.work.is_none() {
        bail!("--workspace is valid only together with --work");
    }
    let work = request
        .work
        .as_deref()
        .map(crate::tmux_work::normalize_work_id)
        .transpose()?;

    if work.is_some()
        && (request.placement != Placement::Pane
            || request.target.is_some()
            || request.name.is_some())
    {
        bail!("--work uses its managed workspace and window; do not combine it with --placement, --target, or --name");
    }

    let requested_cwd = request.cwd.take();
    let initial_cwd_source = requested_cwd
        .clone()
        .map_or_else(std::env::current_dir, Ok)
        .context("resolve current directory")?;
    let initial_cwd = tend::paths::canonicalize(&initial_cwd_source)
        .with_context(|| format!("resolve cwd {}", initial_cwd_source.display()))?;
    if !initial_cwd.is_dir() {
        bail!("cwd is not a directory: {}", initial_cwd.display());
    }

    let workspace = work
        .as_ref()
        .map(|_| match request.workspace.as_deref() {
            Some(workspace) => crate::tmux_work::normalize_workspace_id(workspace),
            None => crate::tmux_work::workspace_id_for_cwd(&initial_cwd),
        })
        .transpose()?;
    let existing_workspace = workspace
        .as_deref()
        .map(crate::tmux_work::find_workspace)
        .transpose()?
        .flatten();
    let existing_work = match (work.as_deref(), workspace.as_deref()) {
        (Some(work), Some(workspace)) => crate::tmux_work::find_work_in(work, Some(workspace))?,
        _ => None,
    };

    let cwd_source = existing_work
        .as_ref()
        .map_or(initial_cwd.as_path(), |existing| existing.cwd.as_path());
    let cwd = tend::paths::canonicalize(cwd_source)
        .with_context(|| format!("resolve cwd {}", cwd_source.display()))?;
    if let (Some(existing), Some(requested)) = (&existing_work, requested_cwd) {
        let requested = tend::paths::canonicalize(&requested)
            .with_context(|| format!("resolve cwd {}", requested.display()))?;
        if requested != cwd {
            bail!(
                "work {} already uses cwd {}; requested {}",
                existing.work,
                cwd.display(),
                requested.display()
            );
        }
    }

    // A session already named after this workspace is this workspace. Making
    // `callabo-2` beside it splits one workspace across two sessions, which
    // is the opposite of what session=workspace means.
    let adopted_session = match (&workspace, &existing_workspace) {
        (Some(workspace), None) => crate::tmux_work::adoptable_session(workspace)?,
        _ => None,
    };
    let created_workspace =
        workspace.is_some() && existing_workspace.is_none() && adopted_session.is_none();
    let created_work = work.is_some() && existing_work.is_none();
    if let Some(existing) = &existing_work {
        request.placement = Placement::Pane;
        request.target = Some(existing.window.clone());
        request.name = Some(existing.window_name.clone());
    } else if let Some(existing) = &existing_workspace {
        request.placement = Placement::Window;
        request.target = Some(existing.session_id.clone());
        request.name = Some(crate::tmux_work::window_name_for_work(
            work.as_deref().expect("managed work has id"),
        )?);
    } else if let Some(session) = adopted_session.as_deref() {
        request.placement = Placement::Window;
        request.target = Some(session.to_string());
        request.name = Some(crate::tmux_work::window_name_for_work(
            work.as_deref().expect("managed work has id"),
        )?);
    } else if let Some(workspace) = workspace.as_deref() {
        request.placement = Placement::Session;
        request.target = None;
        request.name = Some(crate::tmux_work::session_name_for_workspace(workspace)?);
    }

    resolve_placement_target(request, &cwd)?;
    Ok(PreparedLaunch {
        cwd,
        adopted_session,
        workspace,
        work,
        created_workspace,
        created_work,
    })
}

fn resolve_placement_target(request: &mut StartRequest, cwd: &Path) -> Result<()> {
    // new-window rejects a pane id even though pane ids are the stable target
    // Tend exposes to callers. Resolve either a pane or window input to the
    // owning tmux session and let tmux choose an unused window index.
    if request.placement == Placement::Window {
        let target = request
            .target
            .clone()
            .or_else(crate::caller::own_tmux_pane)
            .filter(|target| !target.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("window placement needs --target or this pane's own TMUX_PANE")
            })?;
        let session = resolve_window_session(&target)?;
        // Unnamed, tmux would call the window after the command that started
        // it (`aas`, `zsh`), and with `automatic-rename` off that name sticks.
        if request
            .name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
        {
            let base = agent_window_name(request);
            let listing = crate::surface_names::session_window_listing(&session);
            request.name = Some(crate::surface_names::unique_window_name(&listing, &base));
        }
        request.target = Some(session);
    }
    if request.placement == Placement::Session {
        if request.target.is_some() {
            bail!("session placement does not accept --target");
        }
        let base = request
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map_or_else(
                || {
                    session_base_name(
                        crate::surface_names::project_dir(cwd)
                            .to_string_lossy()
                            .as_ref(),
                    )
                },
                sanitize_session_name,
            );
        let existing = existing_session_names();
        request.name = Some(unique_session_name(base, |candidate| {
            existing.iter().any(|name| name == candidate)
        }));
    }
    Ok(())
}

/// The `new-window -t` target for the session that owns `target`.
///
/// tmux reads a colonless string as a **window** in the caller's current
/// session before it tries it as a session, and tend's own topology makes that
/// collision ordinary rather than unlucky: one session per workspace, whose
/// first window is commonly named after it. Both ends of this function exist
/// for that.
///
/// **Reading the input.** `--target junia` means the session `junia`. Asked
/// plainly, tmux answers with whatever window named `junia` sits in the
/// caller's session — a different session's id, returned without complaint. A
/// trailing colon forces the session reading, so every target is tried that
/// way first and falls back to the plain form. The fallback is what pane and
/// window ids need (tmux rejects `%5:` outright) and what a window *name*
/// target needs; asking tmux which spelling it accepts beats guessing from the
/// first character, which a session named `$work` or a window named `%tmp`
/// would answer wrongly.
///
/// **Writing the output.** The answer is the session id with its own trailing
/// colon: `$7` cannot be re-read as a window name, and the colon means "window
/// unspecified", which is what leaves the index for tmux to choose. A session
/// target without it still carries its current window's index along.
fn resolve_window_session(target: &str) -> Result<String> {
    let mut last_error = None;
    let mut session = None;
    for spelling in [format!("{target}:"), target.to_string()] {
        match session_id_of(&spelling)? {
            Ok(found) => {
                session = Some(found);
                break;
            }
            // Keep the plain form's complaint: it is the one that describes a
            // target tmux does not know at all.
            Err(error) => last_error = Some(error),
        }
    }
    let session = session.ok_or_else(|| {
        let detail = last_error
            .filter(|error| !error.is_empty())
            .map_or(String::new(), |error| format!(": {error}"));
        anyhow::anyhow!("cannot resolve tmux target {target:?}{detail}")
    })?;
    Ok(format!("{session}:"))
}

/// The session id tmux reports for one target spelling, or the message it
/// refused with. The outer `Result` is reserved for not being able to run tmux
/// at all, which is not something another spelling can recover from.
fn session_id_of(target: &str) -> Result<std::result::Result<String, String>> {
    let output = crate::subprocess::output(&mut crate::mux_control::ambient_command_with_args(&[
        "display-message",
        "-p",
        "-t",
        target,
        "#{session_id}",
    ])?)
    .context("resolve tmux window target")?;
    if !output.status.success() {
        return Ok(Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    let session = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if session.is_empty() {
        return Ok(Err(format!(
            "tmux target {target:?} resolved to an empty session"
        )));
    }
    Ok(Ok(session))
}

fn tmux_args(request: &StartRequest, cwd: &Path, command: &str) -> Result<Vec<String>> {
    let cwd = cwd
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("cwd is not valid UTF-8: {}", cwd.display()))?;
    let current_target = || {
        request
            .target
            .clone()
            .or_else(crate::caller::own_tmux_pane)
            .filter(|target| !target.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} placement needs --target or this pane's own TMUX_PANE",
                    match request.placement {
                        Placement::Pane => "pane",
                        Placement::Window => "window",
                        Placement::Session => "session",
                    }
                )
            })
    };

    let args = match request.placement {
        Placement::Pane => {
            let target = current_target()?;
            let split = request.direction.tmux_flag(&target);
            vec![
                "split-window".into(),
                split.into(),
                "-d".into(),
                "-P".into(),
                "-F".into(),
                tend::launch_identity::CREATE_FORMAT.into(),
                "-t".into(),
                target,
                "-c".into(),
                cwd.into(),
                command.into(),
            ]
        }
        Placement::Window => {
            let target = current_target()?;
            let mut args = vec![
                "new-window".into(),
                "-d".into(),
                "-P".into(),
                "-F".into(),
                tend::launch_identity::CREATE_FORMAT.into(),
                "-t".into(),
                target,
            ];
            if let Some(name) = request
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
            {
                args.extend(["-n".into(), name.into()]);
            }
            args.extend(["-c".into(), cwd.into(), command.into()]);
            args
        }
        Placement::Session => {
            if request.target.is_some() {
                bail!("session placement does not accept --target");
            }
            let name = request
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .map_or_else(|| session_base_name(cwd), sanitize_session_name);
            let mut args = vec![
                "new-session".into(),
                "-d".into(),
                "-P".into(),
                "-F".into(),
                tend::launch_identity::CREATE_FORMAT.into(),
                "-s".into(),
                name,
            ];
            let window = match request.work.as_deref() {
                Some(work) => crate::tmux_work::window_name_for_work(work)?,
                None => agent_window_name(request),
            };
            args.extend(["-n".into(), window]);
            args.extend(["-c".into(), cwd.into(), command.into()]);
            args
        }
    };
    Ok(args)
}

/// What a window holding only this agent is called: its task, alias or role,
/// else the agent program. A Work window is named after the Work instead.
fn agent_window_name(request: &StartRequest) -> String {
    crate::surface_names::agent_window_base(
        request.task.as_deref(),
        request.alias.as_deref(),
        request.role.as_deref(),
        request.agent.label(),
    )
}

fn session_base_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "agent".into(), sanitize_session_name)
}

fn sanitize_session_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|ch| if ch == '.' || ch == ':' { '-' } else { ch })
        .collect();
    if cleaned.is_empty() {
        "agent".into()
    } else {
        cleaned
    }
}

fn existing_session_names() -> Vec<String> {
    let mut command = crate::mux_control::ambient_command();
    command.args(["list-sessions", "-F", "#{session_name}"]);
    crate::subprocess::output(&mut command)
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn unique_session_name(base: String, exists: impl Fn(&str) -> bool) -> String {
    if !exists(&base) {
        return base;
    }
    (2..10_000)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !exists(candidate))
        .unwrap_or_else(|| format!("{base}-overflow"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn claude_workers_receive_messages_without_changing_execution_permissions() {
        use tend::collaboration::LaunchPurpose;
        let mut options = vec!["--model".into(), "sonnet".into()];
        super::configure_worker_inbound(
            super::AgentProgram::Claude,
            LaunchPurpose::Worker,
            &mut options,
        );
        let launch = super::AgentProgram::Claude.permission_launch(&options, None, false);
        assert_eq!(
            &launch.args[..2],
            &["--settings", r#"{"crossSessionInbound":"accept"}"#]
        );
        assert!(!launch
            .args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
        let once = options.clone();
        super::configure_worker_inbound(
            super::AgentProgram::Claude,
            LaunchPurpose::Worker,
            &mut options,
        );
        assert_eq!(
            options, once,
            "reconciliation must not stack settings arguments"
        );
        for (agent, purpose) in [
            (super::AgentProgram::Claude, LaunchPurpose::Primary),
            (super::AgentProgram::Codex, LaunchPurpose::Worker),
        ] {
            let mut untouched = vec!["--model".into(), "test".into()];
            let before = untouched.clone();
            super::configure_worker_inbound(agent, purpose, &mut untouched);
            assert_eq!(untouched, before);
        }
    }

    #[test]
    fn claude_worker_keeps_explicit_inbound_settings_and_flag_boundaries() {
        use tend::collaboration::LaunchPurpose;
        for mut options in [
            vec![
                "--settings".into(),
                r#"{"crossSessionInbound":"hold"}"#.into(),
            ],
            vec!["--settings=/operator/settings.json".into()],
        ] {
            let before = options.clone();
            super::configure_worker_inbound(
                super::AgentProgram::Claude,
                LaunchPurpose::Worker,
                &mut options,
            );
            assert_eq!(options, before);
        }
        let mut options = vec!["--".into(), "--settings is prompt text".into()];
        super::configure_worker_inbound(
            super::AgentProgram::Claude,
            LaunchPurpose::Worker,
            &mut options,
        );
        assert_eq!(
            &options[..2],
            &["--settings", r#"{"crossSessionInbound":"accept"}"#]
        );
        assert_eq!(options[2], "--");
    }

    /// A read that sees nothing (a listing that failed, a pane not shown
    /// yet) does not end the look; only the deadline does.
    #[test]
    fn claude_fresh_session_and_explicit_selection_keep_their_semantics() {
        let mut fresh = vec!["--model".into(), "sonnet".into()];
        let expected = claude_session_options(&mut fresh).unwrap();
        assert!(uuid::Uuid::parse_str(&expected).is_ok());
        assert_eq!(fresh.iter().filter(|arg| *arg == "--session-id").count(), 1);
        assert_eq!(claude_session_options(&mut fresh), Some(expected));
        for flags in [
            vec!["--resume", "known"],
            vec!["-r", "known"],
            vec!["--resume=known"],
            vec!["--session-id=known"],
            vec!["--continue"],
            vec!["-c"],
            vec!["--fork-session"],
        ] {
            let mut options = flags
                .iter()
                .map(|flag| (*flag).to_string())
                .collect::<Vec<_>>();
            let before = options.clone();
            claude_session_options(&mut options);
            assert_eq!(options, before);
        }
        let session = uuid::Uuid::new_v4().to_string();
        let mut options = vec!["--resume".into(), session.clone()];
        assert_eq!(claude_session_options(&mut options), Some(session));
    }
    #[test]
    fn each_create_shape_requests_a_tab_delimited_receipt() {
        for placement in [Placement::Pane, Placement::Window, Placement::Session] {
            let mut req = request(AgentProgram::Claude, placement);
            if placement == Placement::Session {
                req.target = None;
            }
            let args = tmux_args(&req, Path::new("/tmp"), "claude").unwrap();
            let at = args.iter().position(|arg| arg == "-F").unwrap();
            assert_eq!(
                args[at + 1],
                "#{pane_id}\t#{pane_pid}\t#{pid}\t#{start_time}"
            );
        }
    }
    #[test]
    fn a_launch_keeps_looking_past_an_empty_read() {
        use std::time::Duration;
        let mut reads = [None, None, Some("codex")].into_iter();
        assert_eq!(
            super::look_for(Duration::from_secs(5), Duration::ZERO, || reads
                .next()
                .flatten()),
            Some("codex")
        );
        let mut seen = 0;
        let started = std::time::Instant::now();
        let never: Option<()> =
            super::look_for(Duration::from_millis(50), Duration::from_millis(5), || {
                seen += 1;
                None
            });
        assert!(never.is_none());
        assert!(seen > 1, "looked {seen} times");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    use super::*;

    /// The socket a launch reserves its alias against is the caller's.
    ///
    /// It used to be `paths::default_socket()` no matter what, so an alias
    /// went to a daemon that did not own the pane — a name taken in the wrong
    /// room while the right one never heard, free to hand it out again.
    /// `scripts/alias-socket-check.py` shows the two panes that results in;
    /// this pins the wiring that carries the socket at all.
    #[cfg(unix)]
    #[test]
    fn only_a_completed_rejected_create_without_a_pane_id_is_definite_failure() {
        use std::os::unix::process::ExitStatusExt;
        let mut output = std::process::Output {
            status: std::process::ExitStatus::from_raw(256),
            stdout: Vec::new(),
            stderr: b"target not found".to_vec(),
        };
        assert!(create_was_rejected(&output));
        output.stdout = b"%42\n".to_vec();
        assert!(
            !create_was_rejected(&output),
            "partial create output must retain uncertainty"
        );
        output.status = std::process::ExitStatus::from_raw(0);
        output.stdout.clear();
        assert!(
            !create_was_rejected(&output),
            "success without a pane id is ambiguous, not a rejection"
        );
    }

    #[test]
    fn agent_start_carries_the_callers_socket() {
        let args = StartArgs {
            purpose: tend::collaboration::LaunchPurpose::Primary,
            agent: AgentProgram::Codex,
            host: LaunchHost::Tmux,
            placement: Placement::Pane,
            target: Some("%9".into()),
            cwd: None,
            prompt: None,
            name: None,
            workspace: None,
            work: None,
            role: None,
            task: None,
            alias: Some("reviewer".into()),
            options: Vec::new(),
            direction: Some(SplitDirection::Right),
            json: false,
        };
        let socket = PathBuf::from("/tmp/somewhere-else.sock");
        let cfg = tend::config::Config::default();
        assert_eq!(StartRequest::from_args(&args, &socket, &cfg).socket, socket);
    }

    /// A plain `tend agent start --agent claude` outside tmux resolves to the
    /// native host. The direction the caller never typed must not refuse it,
    /// while the tmux host keeps splitting right by default.
    #[test]
    fn omitted_direction_starts_natively_and_splits_right_in_tmux() {
        use clap::Parser;
        #[derive(Parser)]
        struct Harness {
            #[command(flatten)]
            start: StartArgs,
        }
        let parse = |argv: &[&str]| {
            Harness::try_parse_from(std::iter::once("start").chain(argv.iter().copied()))
                .unwrap()
                .start
        };
        let plain = parse(&["--agent", "claude"]);
        assert_eq!(plain.direction, None);
        assert_eq!(native_refusal(&plain), None);
        let cfg = tend::config::Config::default();
        let socket = PathBuf::from("/tmp/tend.sock");
        assert_eq!(
            StartRequest::from_args(&plain, &socket, &cfg).direction,
            SplitDirection::Right
        );
        assert_eq!(
            native_refusal(&parse(&["--agent", "claude", "--direction", "auto"])),
            None
        );
        assert!(native_refusal(&parse(&["--agent", "claude", "--direction", "down"])).is_some());
        // The app passes every value as `--flag=value`; clap reads both.
        let joined = parse(&[
            "--agent=codex",
            "--option=--model",
            "--prompt=-x starts with a dash",
        ]);
        assert_eq!(joined.options, ["--model"]);
        assert_eq!(joined.prompt.as_deref(), Some("-x starts with a dash"));
    }

    #[test]
    fn a_capped_pane_is_closed_only_while_nothing_is_typed_in_it() {
        let screen = |composer: &str| {
            format!(
                "> task\n  ⎿  You've hit your limit · resets 3pm (Asia/Seoul)\n─────\n❯ {composer}\n─────\n  ⏵⏵ bypass permissions on"
            )
        };
        assert!(capped_screen_untouched(&screen("")));
        assert!(capped_screen_untouched(&screen(
            "Once fixed, resend your message"
        )));
        // A draft typed while another account was being chosen.
        assert!(!capped_screen_untouched(&screen(
            "wait, use the other repo"
        )));
        // Not the refusal any more, or nothing readable.
        assert!(!capped_screen_untouched(
            "● Working on it\n─────\n❯ \n─────"
        ));
        assert!(!capped_screen_untouched(""));
        // A draft with the frame clipped or missing is not an empty composer.
        assert!(!capped_screen_untouched(
            "  ⎿  You've hit your limit · resets 3pm\n❯ wait, use the other repo\n  ⏵⏵ bypass permissions on"
        ));
        assert!(!capped_screen_untouched(
            "  ⎿  You've hit your limit · resets 3pm\n─────\n❯ wait, use the other repo"
        ));
        // An empty frame further up, with a newer unframed draft below it.
        assert!(!capped_screen_untouched(&format!(
            "{}\n❯ wait, use the other repo",
            screen("")
        )));
        // The limit still on screen while a new turn runs.
        assert!(!capped_screen_untouched(&format!(
            "{}\n✻ Thinking… (2s · esc to interrupt)",
            screen("")
        )));
    }

    #[test]
    fn a_launch_that_did_not_start_carries_its_kept_pane_as_a_field() {
        let error = not_started(
            "did not start: a dialog is waiting; pane %7 is kept for you to answer".into(),
            Some("%7"),
        )
        .context("launch pipeline agent \"review\"");
        let found = not_started_in(&error).expect("typed through the context");
        assert_eq!(found.pane.as_deref(), Some("%7"));
        let json = failure_json(&error);
        assert_eq!(json["ok"], false);
        assert_eq!(json["not_started"]["pane"], "%7");
        assert!(json["error"]
            .as_str()
            .unwrap()
            .starts_with("launch pipeline agent \"review\": did not start"));
        // Any other failure has no `not_started`.
        assert!(failure_json(&anyhow::anyhow!("tmux failed"))
            .get("not_started")
            .is_none());
    }

    fn request(agent: AgentProgram, placement: Placement) -> StartRequest {
        StartRequest {
            creator: None,
            worker_spawn: None,
            bypass_permissions: true,
            purpose: tend::collaboration::LaunchPurpose::Primary,
            socket: tend::paths::default_socket(),
            agent,
            options: Vec::new(),
            placement,
            target: Some("%9".into()),
            cwd: Some(PathBuf::from("/tmp")),
            prompt: Some("review June's changes; don't edit".into()),
            name: None,
            workspace: None,
            work: None,
            role: None,
            task: None,
            alias: None,
            generation: None,
            direction: SplitDirection::Right,
            account: None,
        }
    }

    /// Run real `start()` calls in a child so `HOME`/`PATH`/socket isolation cannot
    /// race other unit tests. All tmux commands target a disposable server.
    /// Fake Claude agents on configured accounts, in a private tmux server:
    /// an `auto` launch that lands on a capped account starts again on the
    /// next one and says why; a pinned one stops with the limit's own words
    /// and keeps its pane.
    #[cfg(all(unix, feature = "aas"))]
    #[test]
    #[allow(clippy::too_many_lines, clippy::unnecessary_debug_formatting)]
    fn a_launch_that_meets_the_limit_moves_on_unless_its_account_is_pinned() {
        use std::process::Command;
        const PROBE: &str = "TEND_TEST_LIMIT_RELAUNCH";
        if let Some(root) = std::env::var_os(PROBE) {
            let root = PathBuf::from(root);
            let profile = |name: &str| tend::accounts::AccountProfile {
                name: name.into(),
                env: [(
                    "CLAUDE_CONFIG_DIR".to_owned(),
                    root.join(name).display().to_string(),
                )]
                .into(),
                env_file: None,
            };
            let launch_with = |name: &str,
                               choice: tend::swap_api::SpawnAccount,
                               homes: &[&str],
                               prompt: Option<&str>| {
                let mut request = request(AgentProgram::Claude, Placement::Session);
                request.prompt = prompt.map(str::to_owned);
                request.socket = root.join("no-daemon.sock");
                request.target = None;
                request.cwd = Some(root.clone());
                request.name = Some(name.into());
                request.account = Some(tend::swap_api::AccountRoute {
                    provider: "claude".into(),
                    choice,
                    aas: "aas".into(),
                    profiles: homes.iter().map(|home| profile(home)).collect(),
                    module_off: false,
                });
                start(request)
            };
            let launch_on = |name: &str, choice: tend::swap_api::SpawnAccount, homes: &[&str]| {
                launch_with(name, choice, homes, Some("task"))
            };
            let launch = |name: &str, choice: tend::swap_api::SpawnAccount| {
                launch_on(name, choice, &["capped", "spare"])
            };
            let started = launch("auto-launch", tend::swap_api::SpawnAccount::Auto)
                .expect("auto starts on the spare account");
            let decision = started.account.clone().unwrap();
            assert_eq!(decision.account.as_deref(), Some("spare"));
            assert_eq!(decision.passed_over, ["capped"]);
            assert_eq!(
                started.input_readiness.as_ref().map(|r| r.state.clone()),
                Some(tend::worker_startup::State::Ready)
            );
            let line = AgentStartOutput::tmux(&started).describe();
            assert!(
                line.contains("on spare because capped was at its usage limit"),
                "{line}"
            );
            let error = launch(
                "pinned-launch",
                tend::swap_api::SpawnAccount::Named("capped".into()),
            )
            .expect_err("a pinned capped account does not start");
            let message = format!("{error:#}");
            assert!(
                message.starts_with("did not start: capped is at its usage limit"),
                "{message}"
            );
            assert!(message.contains("resets 3pm"), "{message}");
            assert!(message.contains("is kept"), "{message}");
            assert!(not_started_in(&error)
                .and_then(|n| n.pane.as_deref())
                .is_some());
            // Auto with no other account: nothing is closed, the pane stays.
            let message = format!(
                "{:#}",
                launch_on(
                    "auto-alone",
                    tend::swap_api::SpawnAccount::Auto,
                    &["capped"]
                )
                .expect_err("no account left")
            );
            assert!(
                message.starts_with("did not start: capped is at its usage limit"),
                "{message}"
            );
            assert!(
                message.contains("not started on another account: every claude account"),
                "{message}"
            );
            // A limit line the transcript does not prove (a tool printed it):
            // never closed and replayed.
            let message = format!(
                "{:#}",
                launch_on(
                    "tool-limit",
                    tend::swap_api::SpawnAccount::Auto,
                    &["toolcap", "spare"]
                )
                .expect_err("an unproven limit is not replayed")
            );
            assert!(
                message.starts_with("did not start: toolcap is at its usage limit"),
                "{message}"
            );
            assert!(message.contains("transcript does not show"), "{message}");
            assert!(message.contains("is kept"), "{message}");
            // A refusal of a prompt tend did not give (none here: the
            // person's own) is never replayed.
            let message = format!(
                "{:#}",
                launch_with(
                    "no-prompt",
                    tend::swap_api::SpawnAccount::Auto,
                    &["capped", "spare"],
                    None
                )
                .expect_err("a launch without tend's prompt is not replayed")
            );
            assert!(message.contains("tend gave it no prompt"), "{message}");
            assert!(message.contains("is kept"), "{message}");
            // An agent that exits at once did not start, and has no pane.
            let message = format!(
                "{:#}",
                launch_on(
                    "exits",
                    tend::swap_api::SpawnAccount::Named("broken".into()),
                    &["broken"]
                )
                .expect_err("an exited agent did not start")
            );
            assert!(
                message.starts_with("did not start: the agent exited"),
                "{message}"
            );
            assert!(!message.contains("is kept"), "{message}");
            let launches = std::fs::read_to_string(root.join("launches")).unwrap();
            let homes: Vec<&str> = launches
                .lines()
                .map(|line| line.rsplit('/').next().unwrap())
                .collect();
            assert_eq!(
                homes,
                ["capped", "spare", "capped", "capped", "toolcap", "capped", "broken"],
                "{launches}"
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let real_tmux = tend::tmux::tmux_binary();
        // Answers its prompt with the limit on the `capped` home, else
        // finishes the turn; either way it then waits at its composer.
        tend::test_support::write_executable(
            &bin.join("claude"),
            format!(
                r#"#!/bin/sh
echo "$CLAUDE_CONFIG_DIR" >> {launches:?}
sid=; prev=
for arg in "$@"; do [ "$prev" = --session-id ] && sid=$arg; prev=$arg; done
log="$CLAUDE_CONFIG_DIR/projects/p/$sid.jsonl"; mkdir -p "${{log%/*}}"
user='{{"type":"user","message":{{"role":"user","content":"task"}}}}'
limit='  \342\216\277  You'"'"'ve hit your limit \302\267 resets 3pm (Asia/Seoul)\n'
printf '> task\n'
case "$CLAUDE_CONFIG_DIR" in
  *broken) sleep 3; exit 1 ;;
  *toolcap)
    # A tool printed the limit line: work was done, nothing proves a refusal.
    printf '%s\n%s\n' "$user" '{{"type":"assistant","message":{{"model":"claude-x","role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{}}}}]}}}}' > "$log"
    printf "$limit" ;;
  *capped)
    printf '%s\n%s\n' "$user" '{{"type":"assistant","message":{{"model":"<synthetic>","role":"assistant","content":[]}},"error":"rate_limit","isApiErrorMessage":true}}' > "$log"
    printf "$limit" ;;
  *) printf '\342\227\217 Done.\n' ;;
esac
printf '\342\224\200\342\224\200\342\224\200\342\224\200\342\224\200\n\342\235\257 \n\342\224\200\342\224\200\342\224\200\342\224\200\342\224\200\n  ? for shortcuts\n'
exec sleep 600
"#,
                launches = root.path().join("launches"),
            ),
            0o755,
        );
        let private_socket = root.path().join("tmux.sock");
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "agent_launch::tests::a_launch_that_meets_the_limit_moves_on_unless_its_account_is_pinned",
                "--nocapture",
            ])
            .env(PROBE, root.path())
            .env("HOME", root.path())
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("TEND_CONFIG", root.path().join("config.toml"))
            .env("TEND_TMUX_SOCKET", &private_socket)
            .env("SHELL", "/bin/sh")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        for key in [
            "RMUX",
            "RMUX_PANE",
            "TMUX",
            "TMUX_PANE",
            "CODEX_THREAD_ID",
            "TEND_HOST",
            "TEND_HOSTS",
            "TEND_SESSION_ID",
            "TEND_SESSION_BACKEND",
        ] {
            child.env_remove(key);
        }
        let output = child.output().unwrap();
        let panes = Command::new(real_tmux)
            .arg("-S")
            .arg(&private_socket)
            .args(["list-panes", "-a", "-F", "#{session_name}"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        // Cleanup runs before assertions, including when the child failed.
        let _ = Command::new(real_tmux)
            .arg("-S")
            .arg(&private_socket)
            .arg("kill-server")
            .output();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // The capped auto launch's pane is gone; the spare one and the
        // pinned launch's pane (kept for the person) remain.
        let mut sessions: Vec<&str> = panes.lines().collect();
        sessions.sort_unstable();
        assert_eq!(
            sessions,
            [
                "auto-alone",
                "auto-launch",
                "no-prompt",
                "pinned-launch",
                "tool-limit"
            ],
            "{panes}"
        );
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::too_many_lines, clippy::unnecessary_debug_formatting)]
    fn duplicate_launch_ticket_refuses_a_second_physical_create() {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixListener;
        use std::process::Command;
        const PROBE: &str = "TEND_TEST_DUPLICATE_LAUNCH";
        if let Some(root) = std::env::var_os(PROBE) {
            let root = PathBuf::from(root);
            let socket = root.join("daemon.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            // Only coordination responses are stubbed; physical creation,
            // archive preparation and duplicate refusal all run for real.
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    let mut line = String::new();
                    std::io::BufReader::new(&stream)
                        .read_line(&mut line)
                        .unwrap();
                    let response = serde_json::json!({
                        "ok": true,
                        "capabilities": [tend::collaboration::HUMAN_PRIORITY_CAPABILITY],
                    });
                    writeln!(stream, "{response}").unwrap();
                }
            });
            let launch_id = uuid::Uuid::new_v4().to_string();
            let mut request = request(AgentProgram::Claude, Placement::Session);
            request.purpose = tend::collaboration::LaunchPurpose::Worker;
            request.socket = socket;
            request.target = None;
            request.cwd = Some(root.clone());
            request.name = Some("duplicate-launch".into());
            request.worker_spawn = Some(tend::collaboration::WorkerSpawnTicket {
                launch_id: launch_id.clone(),
                origin: tend::collaboration::CollaborationOrigin {
                    pane: "%1".into(),
                    socket: None,
                    console: false,
                    agent_session_id: Some("requester".into()),
                    exact_session: true,
                },
            });
            let first = start(request.clone()).expect("first physical launch");
            assert_eq!(first.launch_id.as_deref(), Some(launch_id.as_str()));
            let error = start(request).expect_err("same ticket must never create twice");
            assert!(
                error.to_string().contains("refusing duplicate create"),
                "{error:#}"
            );
            let creates = std::fs::read_to_string(root.join("creates")).unwrap();
            assert_eq!(creates.lines().count(), 1, "{creates}");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let real_tmux = tend::tmux::tmux_binary();
        assert!(Command::new(real_tmux)
            .arg("-V")
            .output()
            .unwrap()
            .status
            .success());
        let wrapper = bin.join("tmux");
        tend::test_support::write_executable(
            &wrapper,
            format!(
                r"#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
if any(arg in ('new-session', 'new-window', 'split-window') for arg in args):
    with open({count:?}, 'a') as file: file.write('create\n')
os.execv({real:?}, [{real:?}] + args)
",
                count = root.path().join("creates"),
                real = real_tmux
            ),
            0o755,
        );
        let claude = bin.join("claude");
        // Draws an idle composer, so the launch's readiness watch ends at once.
        tend::test_support::write_executable(
            &claude,
            "#!/bin/sh\nprintf '\\342\\235\\257 \\n  ? for shortcuts\\n'\nexec sleep 600\n",
            0o755,
        );
        let private_socket = root.path().join("tmux.sock");
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "agent_launch::tests::duplicate_launch_ticket_refuses_a_second_physical_create",
                "--nocapture",
            ])
            .env(PROBE, root.path())
            .env("HOME", root.path())
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("TEND_CONFIG", root.path().join("config.toml"))
            .env("TEND_TMUX_SOCKET", &private_socket)
            .env("SHELL", "/bin/sh")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        for key in [
            "RMUX",
            "RMUX_PANE",
            "TMUX",
            "TMUX_PANE",
            "CODEX_THREAD_ID",
            "TEND_HOST",
            "TEND_HOSTS",
            "TEND_SESSION_ID",
            "TEND_SESSION_BACKEND",
        ] {
            child.env_remove(key);
        }
        let output = child.output().unwrap();
        // Cleanup runs before assertions, including when the child failed.
        let _ = Command::new(real_tmux)
            .arg("-S")
            .arg(&private_socket)
            .arg("kill-server")
            .output();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn a_routed_launch_runs_the_finished_argv_through_aas() {
        let build = || {
            AgentProgram::Codex.permission_launch(&["--no-daemon".into()], Some("it's\nfine"), true)
        };
        let routed = Ok(tend::swap_api::RoutedLaunch::via_aas(
            "/opt/aas".into(),
            "ok@codex".into(),
        ));
        assert_eq!(
            build().on_account(Some(&routed)).argv(),
            [
                "/opt/aas",
                "e",
                "ok@codex",
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                "tui.resume_cwd=\"session\"",
                "--yolo",
                "--no-daemon",
                "it's\nfine"
            ]
        );
        let bare = [
            "codex",
            "-c",
            "check_for_update_on_startup=false",
            "-c",
            "tui.resume_cwd=\"session\"",
            "--yolo",
            "--no-daemon",
            "it's\nfine",
        ];
        assert_eq!(build().on_account(None).argv(), bare);
        assert_eq!(
            build().on_account(Some(&Err("no quota".into()))).argv(),
            bare
        );
    }

    #[test]
    fn codex_profile_expands_cx_and_quotes_the_prompt() {
        assert_eq!(
            AgentProgram::Codex.launch_command(&[], Some("review June's changes; don't edit")),
            "codex -c check_for_update_on_startup=false -c 'tui.resume_cwd=\"session\"' --yolo 'review June'\\''s changes; don'\\''t edit'"
        );
    }

    #[test]
    fn native_launch_keeps_the_prompt_as_one_argv_value() {
        let launch =
            AgentProgram::Codex.native_launch(&[], Some("review June's changes; don't edit"));
        assert_eq!(launch.command, "codex");
        assert_eq!(
            launch.args,
            [
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                "tui.resume_cwd=\"session\"",
                "--yolo",
                "review June's changes; don't edit"
            ]
        );

        let agy = AgentProgram::Antigravity.native_launch(&[], Some("review it"));
        assert_eq!(agy.command, "agy");
        assert_eq!(
            agy.args,
            ["--dangerously-skip-permissions", "-i", "review it"]
        );
    }

    #[test]
    fn the_retired_gemini_cli_is_refused_with_the_reason() {
        for name in ["gemini", "gemini_cli", "Gemini-CLI"] {
            let error = AgentProgram::parse(name).unwrap_err();
            assert!(error.contains("no longer supported"), "{error}");
        }
    }

    #[test]
    fn automatic_workers_do_not_inject_bypass_flags() {
        for program in [
            AgentProgram::Codex,
            AgentProgram::Claude,
            AgentProgram::Antigravity,
        ] {
            let launch = program.permission_launch(
                &["--model".into(), "configured".into()],
                Some("review"),
                false,
            );
            assert!(!launch.args.iter().any(|arg| [
                "--yolo",
                "--dangerously-skip-permissions",
                "yolo",
                "--skip-trust"
            ]
            .contains(&arg.as_str())));
            assert!(launch
                .args
                .windows(2)
                .any(|args| args == ["--model", "configured"]));
        }
        assert!(AgentProgram::Codex
            .permission_launch(&[], None, true)
            .args
            .contains(&"--yolo".into()));
        assert!(
            AgentProgram::Codex
                .permission_launch(&["--yolo".into()], None, false)
                .args
                .contains(&"--yolo".into()),
            "explicit provider options remain authoritative"
        );
    }

    #[test]
    fn configured_options_are_quoted_and_precede_the_prompt() {
        let command = AgentProgram::Codex.launch_command(
            &["--model".into(), "gpt 5; echo nope".into()],
            Some("review $HOME"),
        );
        assert_eq!(
            command,
            "codex -c check_for_update_on_startup=false -c 'tui.resume_cwd=\"session\"' --yolo --model 'gpt 5; echo nope' 'review $HOME'"
        );

        let native = AgentProgram::Antigravity.native_launch(
            &["--model".into(), "gemini-3.7-pro".into()],
            Some("review it"),
        );
        assert_eq!(
            native.args,
            [
                "--dangerously-skip-permissions",
                "--model",
                "gemini-3.7-pro",
                "-i",
                "review it"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_abandoned_native_launch_never_asks_the_daemon_for_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("tendd.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let words: Vec<String> = [
            "agent",
            "start",
            "--json",
            "--agent=codex",
            "--host=native",
            "--direction=auto",
        ]
        .iter()
        .map(|word| (*word).to_string())
        .chain([format!("--cwd={}", dir.path().display())])
        .collect();
        let args = super::parse_start_argv(&words).unwrap();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let scope = crate::subprocess::Scope {
            socket: socket.clone(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            cancel: Some(cancel),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = tend::ipc::Client::new(socket.clone());
        let result = crate::subprocess::with_scope(scope, || {
            runtime.block_on(super::launch_json(
                args,
                &client,
                &socket,
                &tend::config::Config::default(),
                Vec::new(),
            ))
        });
        assert!(result.is_err());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "the daemon was asked to spawn a session"
        );
    }

    #[test]
    fn agent_start_json_uses_one_schema_for_tmux_and_native() {
        let tmux = AgentStartOutput {
            input_readiness: None,
            launch_id: None,
            launch_receipt: None,
            schema_version: 1,
            host: LaunchHost::Tmux,
            agent: AgentProgram::Codex,
            placement: Placement::Pane,
            pane: Some("%7".into()),
            session: Some("$1".into()),
            window: Some("@2".into()),
            name: Some("review".into()),
            workspace: None,
            work: None,
            created_workspace: false,
            created_work: false,
            role: None,
            task: None,
            alias: None,
            cwd: PathBuf::from("/repo"),
            prompt_supplied: true,
            account: None,
        };
        let native = AgentStartOutput {
            host: LaunchHost::Native,
            placement: Placement::Session,
            pane: None,
            session: Some("pty-7".into()),
            window: None,
            ..tmux.clone()
        };
        let tmux = serde_json::to_value(tmux).unwrap();
        let native = serde_json::to_value(native).unwrap();

        let tmux_keys = tmux.as_object().unwrap().keys().collect::<Vec<_>>();
        let native_keys = native.as_object().unwrap().keys().collect::<Vec<_>>();
        assert_eq!(tmux_keys, native_keys);
        assert_eq!(tmux["pane"], "%7");
        assert!(native["pane"].is_null());
        assert_eq!(native["session"], "pty-7");
        assert_eq!(native["placement"], "session");
        assert!(native.get("launch_id").is_none());
        assert!(native.get("launch_receipt").is_none());
    }

    #[test]
    fn auto_launch_uses_the_multiplexer_only_for_a_tmux_or_rmux_shell() {
        assert_eq!(
            LaunchHost::Auto.resolve(Some(HostKind::Tmux)),
            LaunchHost::Tmux
        );
        assert_eq!(LaunchHost::Auto.resolve(None), LaunchHost::Native);
        assert_eq!(
            LaunchHost::Auto.resolve(Some(HostKind::Rmux)),
            LaunchHost::Tmux
        );
        assert_eq!(
            LaunchHost::Auto.resolve(Some(HostKind::Cmux)),
            LaunchHost::Native
        );
        assert_eq!(
            LaunchHost::Native.resolve(Some(HostKind::Tmux)),
            LaunchHost::Native
        );
    }

    #[test]
    fn pane_plan_is_detached_and_returns_the_launch_receipt_format() {
        let request = request(AgentProgram::Codex, Placement::Pane);
        let args = tmux_args(&request, Path::new("/tmp"), "codex --yolo").unwrap();
        assert_eq!(args[0], "split-window");
        assert!(args.iter().any(|arg| arg == "-d"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-F", tend::launch_identity::CREATE_FORMAT]));
        assert!(args.windows(2).any(|pair| pair == ["-t", "%9"]));
        assert_eq!(args.last().unwrap(), "codex --yolo");
    }

    #[test]
    fn window_and_session_plans_use_the_requested_surface() {
        let mut window = request(AgentProgram::Claude, Placement::Window);
        // What `resolve_window_session` hands on: a session id, and a trailing
        // colon that leaves the window index to tmux.
        window.target = Some("$7:".into());
        window.name = Some("review".into());
        let args = tmux_args(&window, Path::new("/tmp"), "claude").unwrap();
        assert_eq!(args[0], "new-window");
        assert!(args.windows(2).any(|pair| pair == ["-n", "review"]));
        assert!(
            args.windows(2).any(|pair| pair == ["-t", "$7:"]),
            "a bare session name would be read as a window in the caller's session: {args:?}",
        );

        let mut session = request(AgentProgram::Antigravity, Placement::Session);
        session.target = None;
        session.name = Some("cal.7041:review".into());
        let args = tmux_args(&session, Path::new("/tmp"), "agy").unwrap();
        assert_eq!(args[0], "new-session");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-s", "cal-7041-review"]));
        // Its first window holds the agent, so it is named for it too —
        // unnamed, tmux would call it after the launch command.
        assert!(args.windows(2).any(|pair| pair == ["-n", "agy"]));

        session.task = Some("Fix login redirect".into());
        let args = tmux_args(&session, Path::new("/tmp"), "agy").unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-n", "fix-login-redirect"]));
    }

    #[test]
    fn managed_workspace_session_names_its_first_work_window() {
        let mut session = request(AgentProgram::Codex, Placement::Session);
        session.target = None;
        session.name = Some("tend".into());
        session.workspace = Some("tend".into());
        session.work = Some("TEST-0001".into());
        let args = tmux_args(&session, Path::new("/tmp"), "codex --yolo").unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-s", "tend"]));
        assert!(args.windows(2).any(|pair| pair == ["-n", "TEST-0001"]));
    }

    #[test]
    fn parsers_accept_short_agent_and_placement_aliases() {
        assert_eq!(AgentProgram::parse("cx").unwrap(), AgentProgram::Codex);
        assert_eq!(Placement::parse(Some("split")).unwrap(), Placement::Pane);
        assert_eq!(
            SplitDirection::parse(Some("vertical")).unwrap(),
            SplitDirection::Down
        );
    }

    #[test]
    fn repeated_session_names_receive_a_stable_suffix() {
        let existing = ["review", "review-2"];
        assert_eq!(
            unique_session_name("review".into(), |name| existing.contains(&name)),
            "review-3"
        );
    }
    #[test]
    fn workers_read_only_the_managed_bootstrap_context_without_changing_modes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("bootstrap.md"), "managed instructions").unwrap();
        let mut options = vec![
            "--permission-mode".into(),
            "default".into(),
            "--".into(),
            "body".into(),
        ];
        configure_worker_bootstrap_context(
            AgentProgram::Claude,
            tend::collaboration::LaunchPurpose::Worker,
            &mut options,
            Some(root.path()),
        );
        assert_eq!(&options[..2], ["--permission-mode", "default"]);
        assert_eq!(
            options[2],
            format!(
                "--add-dir={}",
                tend::paths::canonicalize(root.path()).unwrap().display()
            )
        );
        assert_eq!(&options[3..], ["--", "body"]);
        let before = options.clone();
        configure_worker_bootstrap_context(
            AgentProgram::Claude,
            tend::collaboration::LaunchPurpose::Primary,
            &mut options,
            Some(root.path()),
        );
        assert_eq!(before, options);
    }
    #[test]
    fn codex_bootstrap_does_not_add_write_roots() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("bootstrap.md"), "managed instructions").unwrap();
        let mut options = vec![];
        configure_worker_bootstrap_context(
            AgentProgram::Codex,
            tend::collaboration::LaunchPurpose::Worker,
            &mut options,
            Some(root.path()),
        );
        assert!(options.is_empty());
    }
    #[test]
    fn direct_worker_starts_follow_the_configured_bypass_grant() {
        use clap::Parser;
        #[derive(Parser)]
        struct Harness {
            #[command(flatten)]
            start: StartArgs,
        }
        let args = Harness::parse_from(["start", "--agent", "claude", "--purpose", "worker"]).start;
        let mut config = tend::config::Config::default();
        assert!(
            !StartRequest::from_args(&args, Path::new("/tmp/daemon"), &config).bypass_permissions
        );
        config.collaboration.worker_bypass_permissions = true;
        assert!(
            StartRequest::from_args(&args, Path::new("/tmp/daemon"), &config).bypass_permissions
        );
    }
}
