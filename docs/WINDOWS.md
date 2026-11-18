# Windows

Status: **native, through [rmux](https://rmux.io).** rmux runs natively on
Windows and speaks tmux's command surface over a named pipe, so it is the
pane host tend observes and drives there. `tend` and `tendd` build for
`x86_64-pc-windows-msvc` and install with `cargo install` like anywhere else.
WSL2 remains an option if you would rather run tmux.

This document records what is true today, why, and what the platform gates in
the tree are protecting.

## Install a release

Each release publishes `tend-vX.Y.Z-x86_64-pc-windows-msvc.zip` (`tend.exe`,
`tendd.exe`, `tend-app.exe`, the licences and this page), its checksum
`tend-vX.Y.Z-x86_64-pc-windows-msvc.sha256`, and `install-tend.ps1`. In a normal (not elevated) PowerShell:

```powershell
irm https://github.com/Open330/tend/releases/latest/download/install-tend.ps1 -OutFile install-tend.ps1
powershell -ExecutionPolicy Bypass -File install-tend.ps1
```

It needs no administrator rights and changes only your user:

- checks the zip against the published SHA-256 and stops on a mismatch;
- installs into `%LOCALAPPDATA%\Programs\tend` (or `-InstallDir`, a new or
  empty folder) and adds that folder to
  your user PATH (terminals opened afterwards see it), warning when another
  `tend.exe` comes first on PATH;
- registers a Task Scheduler task, `tend-daemon-<your SID>`, that starts
  tendd at logon (`tend daemon start`, at your normal level, with no console
  window);
- turns on the local dashboard the app talks to
  (`tend init --component dashboard`, which keeps an existing `[dashboard]`);
- starts tendd, and starts `tend-app --install`, which registers its
  notifications and starts it with Windows (the tray menu's *Start with
  Windows* turns that off);
- adds *Tend* to the Start menu;
- records what it created in `install-tend.json` in the install folder.

It changes nothing that belongs to another install: a task, Start menu
entry or start-with-Windows entry that points elsewhere is left alone (or
the install stops before changing anything), and tend commands it runs
ignore a `TEND_SOCKET`, `TEND_CONFIG` or `XDG_*` set only in that shell.

Options: `-Version v0.10.24` for a given release (default the latest),
`-NoApp` for the CLI and daemon only, `-NoShortcut`, `-NoStart`, `-InstallDir`,
and `-Archive <zip>` to install a zip you already have (its `.sha256` next to
it). The installed copy remembers its folder and whether it has the app and
the shortcut. It warns when rmux is missing (`winget install Helvesec.RMUX`); tendd
runs without it but sees no panes. Then run `tend init` from an rmux pane for
agent hooks and key bindings (see *Native install* below).

**Upgrade**: run the installed copy again, with `-Version` for anything
other than the latest. It stops tend-app and tendd from the install folder,
replaces the binaries and starts both:

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\tend\install-tend.ps1"
```

`tend upgrade` still builds from a source checkout and does not update a
release install.

**Uninstall**: `tend init --uninstall` first if you ran `tend init` (it removes
the hooks and rmux bindings), then

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\tend\install-tend.ps1" -Uninstall
```

which removes what `install-tend.json` lists: the start-with-Windows entry
and notification registration (when they point at this install), tendd and
tend-app, the logon task, the Start menu entry, the PATH entry, the
installed files, and the folder if nothing else is in it. Settings and data stay in
`%APPDATA%\tend` and `%LOCALAPPDATA%\tend`; delete those by hand to start
over.

**Not code-signed.** The binaries carry no signature, so Windows treats them
as from an unknown publisher. What that looked like on a clean Windows 11
(checked 2026-11-19):

- Through the script, nothing is shown: `Invoke-WebRequest` does not mark
  files as downloaded from the internet, and the script unblocks the
  installed files.
- A zip downloaded in a browser and unpacked in Explorer keeps that mark,
  and opening `tend-app.exe` from it shows Microsoft Defender SmartScreen's
  *Windows protected your PC: Microsoft Defender SmartScreen prevented an
  unrecognized app from starting*. *More info* then *Run anyway* starts it;
  installing through the script avoids the prompt.
- Smart App Control was in evaluation mode there and blocked nothing. When
  it is **on** (enforcement), it lets an app run only if Microsoft's
  intelligence recognises it as safe or it is signed by a trusted
  publisher, so an unrecognised unsigned Tend build may be blocked, with no
  per-app override. Enforcement mode was not tested.
- A script downloaded in a browser is marked too, so PowerShell's default
  `RemoteSigned` refuses it; `-ExecutionPolicy Bypass` above runs it for that
  one call.

There is no winget package yet.

## Native install (rmux)

From source:


```powershell
winget install Helvesec.RMUX          # or any rmux >= 0.10 on PATH (see below)
cargo install --path crates/tendd --locked
cargo install --path crates/tend-cli --locked
tend init --component tmux-popup,tmux-peek,claude-hooks,codex-hooks,tendd-shellrc
```

### On a new machine

What a clean Windows 11 Pro install (26H2, checked 2026-11-16) lacks before
the commands above work:

- **A C++ toolchain.** `cargo install` needs the Visual Studio 2022 Build
  Tools with only *MSVC x64/x86 build tools* and a *Windows 11 SDK*
  (about 4.3 GB), then [rustup](https://rustup.rs) (MSVC host) and Git.
  A release build of `tendd`, `tend` and `tend-app` takes about 18 minutes
  on 4 cores and about 2 GB of `target`.
- **rmux.** `winget install Helvesec.RMUX` works from a signed-in terminal
  (the first winget run asks to accept the `msstore` source terms); the
  release's Windows zip works too. winget is not available to a service or
  `SYSTEM` session.
- **PowerShell 7.** Windows ships only Windows PowerShell 5.1, whose
  default execution policy is `Restricted` (no profile runs). The
  `tendd-shellrc` autostart is written to the PowerShell 7 profile, so
  install PowerShell 7 (its policy was `RemoteSigned` on the tested machine;
  if the profile does not load, see `Get-ExecutionPolicy -List`). On the clean machine
  an rmux server's `default-shell` was `cmd.exe` (from `%COMSPEC%`), which
  reads no profile: set `set -g default-shell pwsh` in `~/.rmux.conf` for
  the autostart to run.
- **Nothing for the app itself.** WebView2 ships with Windows 11 (157 here).
  An OpenSSH server is needed only when another machine reaches this one as
  a Fleet host.

`tend init` writes its bindings to `~/.rmux.conf` on Windows — rmux reads that
file (then `%APPDATA%\rmux\rmux.conf`) and never `~/.tmux.conf` — and accepts
rmux in place of tmux in its pre-flight. Run it from inside an rmux pane so the
live reload reaches the right server.

`tendd-shellrc` is the autostart: on Windows it adds a block to the PowerShell
profile (`$PROFILE.CurrentUserCurrentHost`), so the first pane rmux opens
starts the daemon. `tend daemon start` creates the daemon through
`Win32_Process.Create` (see *Detaching the daemon* below), so it survives the
shell — and the rmux server — that started it. It writes its log itself
(`tendd --log-file`) to `%TEMP%\tendd.log`, which `tend daemon logs` reads.

rmux 0.10.0 redraws a whole popup every frame, so the watch popup flickers.
[Open330/rmux](https://github.com/Open330/rmux)'s `main` carries the renderer
fix ([RMUX.md](RMUX.md#popup-rendering-on-rmux-0100)); until a release does,
`cargo install --git https://github.com/Open330/rmux --locked rmux` builds it.

### A scratch daemon

A tendd started with its own XDG directories (a test or a second, scratch
daemon) is not the user's daemon, so it never infers a multiplexer from
`$RMUX` or rmux's default server: those are the user's. Give it a private
server to observe with `TEND_RMUX_SOCKET`, the server's pipe (for example
`rmux -L scratch display-message -p '#{socket_path}'`) or a pane's `$RMUX`
value. It is the rmux counterpart of a scratch `TMUX_TMPDIR`. Only a
process restricted to its own multiplexer reads it (such a daemon, or a CLI
aimed at one); the user's daemon and an ordinary CLI ignore it. A scratch daemon
does not sample activity from that server (activity sampling stays
fail-closed for it).

### After a reboot

tendd snapshots the workspace on an interval, and follows the server across
restarts even though its pipe name changes.

```powershell
tend init --component restore-at-login
```

registers a Task Scheduler task, `tend-restore`, that runs `tend restore
--boot` at logon: when the rmux server has no sessions it starts it and
rebuilds the newest snapshot, resuming the agents, and otherwise it changes
nothing. Output goes to `%TEMP%\tend-restore.log`. The task runs at the
level of the terminal that installed it — an elevated terminal's rmux server
is a different server from a limited one's — and starts rmux through
`Win32_Process.Create`, since a task's job object forbids the breakaway rmux
needs for its daemon. By hand, `tend restore --run` does the same into a
running server.

Verified on Windows 11 with rmux 0.10.0 and Open330/rmux `main` by driving a
real client:

- prefix `s`/`S`/`D`/`q`/`M` open watch, Fleet watch, the dashboard, peek and
  agent move in popups; Enter in watch jumps the client across sessions;
  agent move relocates a pane.
- hooks and the Claude status line attach agents to their rmux panes;
  `tend status`, `recap`, `msg` (peer delivery and inbox) and `snapshot` work.
- `tend restore` rebuilds sessions, windows, panes and directories, relaunches
  their commands, and resumes Claude with `--resume <id>`.
- `tend agent start` places an agent in an rmux pane or window.
- `tend daemon start/stop/restart/logs` and the PowerShell profile autostart.
- `tend agent start --agent codex` launches Codex (an npm `.cmd` shim) in a
  window, and its hooks track it. Codex asks once to trust the hooks
  `tend init` added.
- `tend upgrade` end to end: build, swap the running binaries, re-exec tendd.
- SSH Fleet with Windows as the controller (Windows OpenSSH): `fleet host
  doctor`, `status`, `panes`, `capture`, `send`, `attach` and `fleet watch`
  against a Linux host. A WSL host on `localhost` needs `HostName 127.0.0.1`:
  WSL forwards localhost over IPv4 only.
- Dashboard pane sharing's whole flow — OIDC sign-in, share link, guest
  prompt, reconnect, daemon restart, revocation — through
  `scripts/dashboard-sharing-smoke.py` on a private rmux label.
- `tend restore --boot` from a logon task, into a stopped server.
- `tend sidebar show` on a private rmux label: the sidebar opens, and its
  `after-new-window`, `after-select-window` and `after-resize-pane` hooks
  give a new or newly selected window its sidebar and turn a native zoom
  into the focus layout. rmux 0.10.0 accepts every hook the sidebar sets
  (`after-select-window`, `after-new-window`, `client-session-changed`,
  `after-resize-pane`, `after-select-pane`, each with the `[9105]` index)
  and lists them in `show-hooks -g`. `client-session-changed` and the
  prefix `z`/`e` keys need an attached client and were not driven.

### Known gaps

- **Codex MCP live reload** needs Codex's Unix control socket, which Codex
  does not create on Windows; `tend mcp reload` reports it unavailable.
- **Collaboration provenance** records no caller pid: a named pipe has no
  safe peer-credential call, so requests read "origin unverified".
- **`tend onboard`'s live tour** builds a POSIX-shell sandbox on a private
  tmux server and is not expected to work.
- **`tend fleet attach`** through Windows OpenSSH can let a remote tmux's
  terminal-query reply (`…;42c`) reach the remote shell as typed text.
- **`tend upgrade`** needs a source checkout on Windows; a release install
  upgrades by running `install-tend.ps1` again. It moves the running `tend.exe`/`tendd.exe` aside
  before `cargo install`, since Windows refuses to overwrite a running
  executable but allows renaming one.
- Re-exec on upgrade spawns a successor and exits instead of `exec`ing in
  place, so the daemon's pid changes.
- **Configured accounts** (`[[accounts.claude]]`, `[[accounts.codex]]`) are
  refused: launching and relaunching through one builds an `env K=V …`
  prefix for `sh` (`account_route.rs`, `account_swap.rs`
  `profile_relaunch_line`). Accounts reached through `aas` are unaffected.
- **`tend init --all-homes`** finds no running agents' homes: the scan of
  live agents' `CLAUDE_CONFIG_DIR`/`CODEX_HOME` reads `ps` and is Unix-only,
  so only the active home and configured accounts' homes get the MCP server.
- **Stale `tend mcp` detection** cannot read which binary a running server
  was started from (`lsof` on Unix), so it judges staleness by start time
  against the installed binary alone.
- **`tend mcp` does not follow upgrades**: on Unix it re-executes itself when
  its binary is replaced; on Windows it keeps the old build until the agent
  restarts it.
- **Claude's folder-trust prompt** after a relaunch or swap is now parsed for
  a drive or UNC path, but what Claude prints there on Windows has not been
  captured; until it has, expect to answer that prompt by hand.

## Tend for Windows (the app)

`tend-app.exe` (`crates/tend-app`) is the Windows counterpart of Tend for
Mac's menu bar and Home: an icon in the notification area with the Tend
mark (amber when something needs you, red when the dashboard cannot be
reached), a Home window, toasts, and start with Windows. Design and its
security review: [proposals/WINDOWS_APP.md](proposals/WINDOWS_APP.md).

`install-tend.ps1` (see *Install a release*) sets all of this up. From
source:

```powershell
tend init --component dashboard       # the app talks to the local dashboard
cargo install --path crates/tend-app --locked
tend-app --install                    # start with Windows, register toasts, start
```

- **Home** is the dashboard's `/home` page (Your turn / In progress /
  Finished, every Fleet host), in a WebView2 window. Answering and deciding
  go through the same audited routes as the web page (`web:token`).
  Closing the window frees WebView2; the app stays in the tray.
- **Toasts**: one per new question, waiting agent or result to review, on
  any host; several at once, or what is already waiting at start, are one
  summary. A toast goes away when its item is decided anywhere (after a
  complete read of its host). Clicking one opens Home on that card.
- **Tray menu**: Open Home, Notifications, Start with Windows, Start tendd
  (when the dashboard does not answer; the app also tries it once at
  start), Quit.
- `tend-app --open ITEM` opens Home on a card; a second start hands its
  request to the running app. `tend-app --uninstall` removes the start entry
  and the toast registration.

How it talks to tendd: only to the loopback dashboard, with the dashboard
token read from tendd's config (or `TEND_DASHBOARD_TOKEN`), and only after
the server proved it knows the token (`GET /api/proof`, bound to this
host's node id and the dashboard address). The page never sees the token:
the window loads Home from the app's own origin and the app forwards only
Home's reads (the board from `GET /api/home`, so Work units, helpers and the
backlog show as in the browser) and its audited writes: answering or
deciding an item and marking a finished unit looked at. Explore, Settings,
pane links and Start Work stay hidden in the window (it does not forward
`/api/access`, and those need pane prompts, work control or operator
administration); open the dashboard in a browser for them. It refuses a dashboard without
token auth, a token under 32 characters, or a non-loopback `bind`, and the
tray says which.

State (`%LOCALAPPDATA%\tend\app`): `app.log`, the items already told about
(`app-state.json`), the WebView2 profile. `TEND_APP_STATE_DIR`,
`TEND_APP_ID` and `TEND_APP_RUN_VALUE` move all of it for a test run.

`scripts/windows-app-e2e.ps1 -Bin <dir> -Desktop` is the end-to-end test:
two scratch daemons on private rmux servers (one a Fleet host over
`ssh localhost`), seeded questions and results, the app started by its Run
command, toasts, Home driven through UI Automation, answers and decisions
read back from both daemons, and an impostor on the dashboard port that
must not see the token.

## Use WSL2

```bash
# inside your WSL distribution
curl -fsSL https://raw.githubusercontent.com/Open330/tend/main/scripts/install.sh | sh
```

Requirements are the ordinary Linux ones: tmux, and Rust `1.89+` to build.
Windows Terminal attaches to the WSL distribution like any other shell, so
`tend watch` runs in a Windows Terminal tab with no bridge in between.

Clone into the WSL filesystem (`~/`) rather than building from `/mnt/<drive>`.
The 9p mount that backs `/mnt` makes cargo builds several times slower.

## What stood in the way

Five independent reasons. Each is resolved; each is kept here because what it
cost and what it could not recover are the useful part of the record.

### 1. No host to observe — resolved by rmux

Native rmux implements the tmux command surface on Windows, and tend's rmux
backend drives it through that CLI. The reasoning below is still true of
Windows Terminal.

[`PaneBackend`] requires `list_panes`, `resolve_pane`, `capture_pane`,
`pane_pid_map`, `current_pane`, `focus_pane`, and `send_text`. Windows Terminal
provides none of them: `wt.exe` accepts split-pane arguments at launch and
returns, with no pane enumeration, no stable pane ids, no screen capture, and
no way to send input to a pane it is not focused on. ConPTY is a pseudoconsole
API, not a multiplexer.

### 2. IPC was a Unix-domain socket — resolved

`ipc.rs` used to bind a `UnixListener` and thread
`tokio::net::unix::OwnedWriteHalf` through handler signatures. It now goes
through `crates/tend/src/transport.rs`, which resolves to a Unix socket or a
Windows named pipe. See [PROTOCOL.md](../PROTOCOL.md#transport).

What did not port cleanly:

- **Protection is weaker.** `0600` on a socket file has no named-pipe
  equivalent. A pipe's DACL is fixed at creation and setting a custom one needs
  an unsafe raw call `unsafe_code = "forbid"` rules out, so the pipe inherits
  the creating token's default DACL — an administrator can connect.
  `first_pipe_instance` and `reject_remote_clients` cover name squatting and
  SMB reachability, which are the other two exposures.
- **No peer credentials.** `SO_PEERCRED` has no safe counterpart, so
  collaboration provenance records `None` for pid/uid/gid on Windows.
- **The synchronous client** (`ipc::blocking_call`, used to register a
  launched agent and reserve its alias) cannot bound a read on a pipe opened
  as a file. On Windows it runs the async transport under a timeout on a
  thread of its own instead.
- **Accept has a gap a socket does not have.** One pipe instance serves one
  client, and the successor is created only once the pending instance is
  claimed. A client connecting in that window gets `ERROR_PIPE_BUSY`, so
  clients retry it briefly; a socket's backlog hides the same burst.

### 3. No process source — resolved through sysinfo

Linux reads `/proc`; other Unix hosts take one `ps` snapshot. Windows has
neither, and `unsafe_code = "forbid"` rules out calling Toolhelp32 directly.
The Windows arms read the same table through `sysinfo`, which wraps those APIs
safely:

| File | Windows arm |
| --- | --- |
| `process_snapshot.rs` | whole table (pid, parent, argv), one process's argv, start time, liveness |
| `adapters/proc_ancestry.rs` | parent lookup for hook ancestry |
| `state.rs` | pid liveness for `tend proc`/`tend run` rows |
| `account_swap.rs` | the agent's command line; `taskkill` in place of signals |
| `tend-cli` `stale_agents.rs`, `reload/process.rs` | stale `tend mcp` scan; each pane's command for snapshots |

rmux reports `#{pane_pid}` (the pane's shell) but, a console having no
foreground process group, always the shell as `#{pane_current_command}`. The
program running in a pane is therefore read from the shell's children:
snapshot records it (an agent child wins over a helper such as
`WindowsPackageManagerServer.exe`), and restore confirms a relaunch from it.

### 4. Commands are read by PowerShell, not `/bin/sh` — resolved

tmux runs `new-window`, `split-window`, `display-popup` and `run-shell`
commands through `/bin/sh`. rmux on Windows runs them through its
`default-shell`, PowerShell by default, where POSIX quoting (`'\''`), `exec`,
`2>/dev/null`, `|| true`, `printf`/`read`, and a leading quoted program are
all errors. A popup whose command fails closes at once under `-E`, which is
why prefix `s` appeared to do nothing.

`tend::shell::Dialect` is the single place a command line is built:
PowerShell quoting doubles `'`, a program that needs quoting goes through the
call operator `&`, `exec` is dropped, and `cd` is `Set-Location -LiteralPath`.
Popups, agent launch, restore, account swap, watch's work-up window and spawn
form all go through it. `tend::shell` also names programs without their path
or `.exe`/`.cmd`, recognises `pwsh`/`powershell`/`cmd` as shells, runs scripts
(`work up`'s `prepare`, the status-line `--forward`) through PowerShell, and
finds npm's `.cmd` shims, which `Command::new` alone does not.

A shell line nested inside a multiplexer command — the sidebar's
`if-shell … 'run-shell -b …'` hooks — is quoted twice, each time for the
parser that reads it: `Dialect` for the shell, `tend::shell::mux_quote` for
tmux/rmux. Quoting both layers in `PowerShell`'s dialect lost one: rmux reads
`'a''b'` as `ab`, and stored the hook as `run-shell -b & C:\…\tend.exe …`,
so new windows never got a sidebar.

### 5. Unix process management — resolved

- **Detaching the daemon.** `nohup` does not exist, and a process created
  with `CreateProcess` inherits every inheritable handle of its creator — so a
  daemon started by `tend daemon start *> $null` held the pipe PowerShell
  reads from, and the profile hook would have waited for the daemon to exit.
  It also stayed in the launching pane's job object, which rmux closes with
  its server. `tend daemon start` therefore creates tendd through
  `Win32_Process.Create`: the WMI service inherits nothing to it and puts it
  in no job.
- **Signals.** `tendd` installs Ctrl+C / console-close / shutdown handlers in
  place of SIGTERM and SIGINT, and re-exec spawns a successor rather than
  calling `exec`.
- **The ambient multiplexer** is rmux's default server on Windows when the
  shell is in no pane, as tmux's is elsewhere; Fleet Work runs there.
- **Paths.** `std::fs::canonicalize` returns verbatim paths (`\\?\F:\…`) that
  no hook reports; `tend::paths::canonicalize` drops the prefix so cwd
  comparisons hold.

Windows also gives the main thread a 1 MiB stack. An unoptimized build of the
CLI overflows it before `tend init --help` prints, so `.cargo/config.toml`
links MSVC targets with the 8 MiB stack Unix hosts default to.

## What builds and runs today

`tend`, `tendd`, and `tend-cli` build for `x86_64-pc-windows-msvc` with no
clippy warnings. Tests that exercise Unix-only behaviour — `#!/bin/sh`
fixtures through `test_support::write_executable`, `0600`/`0700` mode
assertions, and socket send-buffer backpressure — are gated `#[cfg(unix)]`.

`cargo test -p tend-cli -p tendd -p tend-work` passes on Windows (checked
2026-11-16 on a clean Windows 11 VM with rmux 0.10.0 and Windows PowerShell:
`tend-cli` 1,235 unit and 13 integration tests, `tendd` 166, `tend-work`
156). Build the binaries first (`cargo build --bins -p tend-cli -p tendd`):
the integration tests run them. The live rmux tests start private servers by
label, outside the caller's job, so they run over SSH too.

In `cargo test -p tend --lib` the last count was 23 of 1,229 failing, on Unix
assumptions in fixtures: paths like `/tmp/...` that are not absolute without
a drive prefix, so request validation rejects them, and bounded-command
tests that run `sh`. CI's `windows` job (windows-latest, rmux 0.10.0 from
its release zip) runs clippy over the workspace and the tests of `tend-cli`,
`tendd`, `tend-work` and `tend-app`; the desktop end-to-end test of the app
(`scripts/windows-app-e2e.ps1 -Desktop`) stays a manual check before a
release, since it needs a logged-on desktop.

## What is left

Teaching `tend`'s own test fixtures a second path grammar, so its unit tests
can join the Windows job; the gaps listed above.

[`PaneBackend`]: ../crates/tend/src/backend/mod.rs
