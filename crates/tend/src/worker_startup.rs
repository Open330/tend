//! Mechanical preparation of an explicitly launched worker, before any task is sent.
//! The caller must fence the new process, pane and authorized working directory.
//! No conversation prompt, login choice or tool permission is answered here.
use crate::agent_tui::{
    self as io, composer_draft, launch_blocker, option_rows, pending_dialog, turn_running,
    type_keys, wait_for_resume_answering_with_probe, Clock, Consent, LaunchBlocker, Resume,
    READY_QUIET, STARTUP_DIALOGS,
};
use crate::event::AgentKind;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ready,
    Blocked,
    Exited,
    Unsettled,
    /// The account is at its usage limit: the agent runs but cannot work.
    Limited,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    pub answered: Vec<String>,
    pub reason: Option<String>,
}

// Claude inserts hard line breaks in a narrow pane, so tmux's -J cannot
// repair the two wrapped option labels. Normalize only an exact two-row
// folder-trust menu and its exact footer, never arbitrary prose or approvals.
fn normalize_capture(screen: String) -> String {
    let lines: Vec<_> = screen.lines().collect();
    let compact = |text: &str| {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    let Some(footer) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("Enter"))
    else {
        return screen;
    };
    if compact(&lines[footer..].join("")) != "Entertoconfirm·Esctocancel" {
        return screen;
    }
    let mut end = footer;
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let start = lines[..end]
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map_or(0, |i| i + 1);
    let menu = compact(&lines[start..end].join(""));
    let yes = "Yes, I trust this folder";
    for no in ["No, exit", "No, exit Claude Code"] {
        for (first, second) in [(no, yes), (yes, no)] {
            for selected_first in [true, false] {
                let expected = if selected_first {
                    format!("❯{first}{second}")
                } else {
                    format!("{first}❯{second}")
                };
                if menu == compact(&expected) {
                    return if selected_first {
                        format!(" ❯ {first}\n   {second}\n Enter to confirm · Esc to cancel")
                    } else {
                        format!("   {first}\n ❯ {second}\n Enter to confirm · Esc to cancel")
                    };
                }
            }
        }
    }
    screen
}

// Custom statusLine replaces Claude's default prompt hints. This probe is
// worker-only: account-swap resume must never accept an old composer's frame.
// Restrict the actual composer to the bottom of the screen, leaving at most
// three footer lines and rejecting shell/relaunch output underneath it.
fn claude_composer_visible(screen: &str) -> bool {
    let tail = crate::agent_tui::screen_tail(screen, 12);
    let lines: Vec<_> = tail.lines().collect();
    let rules: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (line.chars().count() >= 5 && line.chars().all(|c| c == '─')).then_some(index)
        })
        .collect();
    let [.., top, bottom] = rules[..] else {
        return false;
    };
    let footer: Vec<_> = lines[bottom + 1..]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    footer.len() <= 3
        && !footer.iter().any(|line| {
            line.starts_with(['$', '❯', '›', '>', '#', '%'])
                || line.starts_with("claude ")
                || line.starts_with("exec ")
                || line.contains(" $ ")
                || line.contains(" % ")
                || line.contains(" --resume")
                || line.contains(" --session-id")
        })
        && lines[top + 1..bottom]
            .first()
            .is_some_and(|line| line.trim_start().starts_with('❯'))
}

/// Reuses the account-switch startup parser and settling rules. `alive` must
/// check the original launch identity, not merely that a pane still exists.
/// The permission profile is retained: bypass consent is possible only when
/// the caller explicitly launched with it. Folder settings remain a blocker.
pub fn prepare(
    kind: AgentKind,
    capture: impl FnMut() -> Option<String>,
    alive: impl FnMut() -> bool,
    send_key: impl FnMut(&str) -> bool,
    bypass_requested: bool,
    fresh: bool,
    timeout: Duration,
) -> Report {
    if !matches!(kind, AgentKind::ClaudeCode | AgentKind::Codex) {
        return Report {
            state: State::Blocked,
            answered: vec![],
            reason: Some("provider has no verified startup/composer probe".into()),
        };
    }
    prepare_with(
        &io::RealClock,
        kind,
        capture,
        alive,
        send_key,
        bypass_requested,
        fresh,
        timeout,
        READY_QUIET,
        Duration::from_millis(250),
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_with(
    clock: &dyn Clock,
    kind: AgentKind,
    mut capture: impl FnMut() -> Option<String>,
    mut alive: impl FnMut() -> bool,
    mut send_key: impl FnMut(&str) -> bool,
    bypass_requested: bool,
    fresh: bool,
    timeout: Duration,
    quiet: Duration,
    tick: Duration,
) -> Report {
    let last = std::cell::RefCell::new(String::new());
    let mut answered = Vec::new();
    let resume = wait_for_resume_answering_with_probe(
        clock,
        || {
            let raw = capture()?;
            let screen = if kind == AgentKind::ClaudeCode {
                normalize_capture(raw)
            } else {
                raw
            };
            last.replace(screen.clone());
            Some(screen)
        },
        &mut alive,
        |keys| {
            // Only initial setup, never resume an old goal or answer a task's
            // approval. A quoted/ambiguous dialog is rejected by the parser.
            let allowed = may_answer(kind, &last.borrow(), bypass_requested);
            allowed && type_keys(clock, &mut send_key, keys)
        },
        &|consent| consent == Consent::None || (bypass_requested && consent == Consent::Bypass),
        timeout,
        quiet,
        tick,
        &mut answered,
        &|screen| {
            crate::agent_tui::agent_prompt_visible(screen)
                || (kind == AgentKind::ClaudeCode && claude_composer_visible(screen))
        },
    );
    // A fresh launch's account cannot work (limit) or is not signed in:
    // whatever else the screen shows follows from that. A resumed one's
    // history can show the same lines, so it is never read for them.
    let (state, reason) = match resume {
        _ if fresh && launch_blocker(&last.borrow()).is_some() => {
            blocked_launch(launch_blocker(&last.borrow()).expect("just matched"))
        }
        // Codex asks before running hooks that are new or changed. Accepting
        // lets them run outside the sandbox, so it stays the user's call.
        _ if kind == AgentKind::Codex && codex_hook_review(&last.borrow()) => (
            State::Blocked,
            Some(
                "Codex asks to review new or changed hooks before it starts; answer it in the \
                 pane (tend does not trust hooks for you), then prepare the worker again"
                    .into(),
            ),
        ),
        Resume::Ready
            if turn_running(&last.borrow())
                || composer_draft(&last.borrow(), kind)
                    .is_some_and(|text| !(kind == AgentKind::Codex && cut_placeholder(&text))) =>
        {
            (
                State::Blocked,
                Some("worker is busy or its composer contains a draft; nothing was cleared".into()),
            )
        }
        Resume::Ready => (State::Ready, None),
        Resume::Blocked(reason) => (State::Blocked, Some(reason)),
        Resume::Exited => (
            State::Exited,
            Some("the original worker process or working directory changed".into()),
        ),
        Resume::Unsettled => (
            State::Unsettled,
            Some("no stable agent input prompt observed before the deadline".into()),
        ),
    };
    Report {
        state,
        answered: answered.into_iter().map(str::to_owned).collect(),
        reason,
    }
}

/// How long a launched agent's first turn must run before the launch
/// counts as started: a capped account answers the prompt with its limit
/// line within a second or two of the spinner.
pub const WORKING_PROOF: Duration = Duration::from_secs(4);

/// Watch an agent tend just launched with its first prompt on the command
/// line, until it is working on it or cannot. The agent holds that prompt
/// until its startup dialogs are answered, so nothing is ever typed into a
/// dialog. The same dialogs as [`prepare`] are answered, under the same
/// rules; the rest are left for the person.
///
/// Ready means the prompt's turn has run for [`WORKING_PROOF`], or ended
/// at a composer without a limit or sign-in line above it.
pub fn watch_prompted(
    kind: AgentKind,
    capture: impl FnMut() -> Option<String>,
    alive: impl FnMut() -> bool,
    send_key: impl FnMut(&str) -> bool,
    bypass_requested: bool,
    fresh: bool,
    timeout: Duration,
) -> Report {
    if !matches!(kind, AgentKind::ClaudeCode | AgentKind::Codex) {
        return Report {
            state: State::Blocked,
            answered: vec![],
            reason: Some("provider has no verified startup/composer probe".into()),
        };
    }
    watch_prompted_with(
        &io::RealClock,
        kind,
        capture,
        alive,
        send_key,
        bypass_requested,
        fresh,
        Timing {
            timeout,
            quiet: READY_QUIET,
            working: WORKING_PROOF,
            tick: Duration::from_millis(250),
        },
    )
}

/// The waits of [`watch_prompted`], so tests can shorten them.
#[derive(Debug, Clone, Copy)]
struct Timing {
    timeout: Duration,
    quiet: Duration,
    working: Duration,
    tick: Duration,
}

const EXITED: &str = "the agent exited during startup";

/// What a prompted launch's settled screen calls for.
enum Step {
    /// Nothing yet: keep watching.
    Wait,
    /// A startup dialog tend may answer.
    Answer(&'static io::Dialog),
    /// The watch is over.
    Done(State, Option<String>),
}

/// The step for a screen that has held still: a dialog to answer or stop
/// at, the agent's composer (its turn ran and ended), or nothing yet.
fn settled_step(
    kind: AgentKind,
    screen: &str,
    answered: &[&'static str],
    bypass_requested: bool,
    may_linger: bool,
    alive: &mut dyn FnMut() -> bool,
) -> Step {
    let blocked = |reason: String| Step::Done(State::Blocked, Some(reason));
    if kind == AgentKind::Codex && codex_hook_review(screen) {
        return blocked(
            "Codex asks to review new or changed hooks before it starts; answer it in the pane \
             (tend does not trust hooks for you)"
                .into(),
        );
    }
    if let Some(dialog) = pending_dialog(screen, STARTUP_DIALOGS, answered) {
        if !may_answer(kind, screen, bypass_requested) {
            return blocked(format!(
                "a startup dialog needs your answer ({}); nothing was typed",
                dialog.label
            ));
        }
        return if alive() {
            Step::Answer(dialog)
        } else {
            Step::Done(State::Exited, Some(EXITED.into()))
        };
    }
    if may_linger && io::answered_dialog_lingers(screen, answered) {
        // Answered already; the agent has not redrawn yet.
        return Step::Wait;
    }
    if let Some(line) = io::open_question(screen) {
        return blocked(format!("a dialog is waiting for an answer ({line})"));
    }
    if !alive() {
        return Step::Done(State::Exited, Some(EXITED.into()));
    }
    if io::agent_prompt_visible(screen)
        || (kind == AgentKind::ClaudeCode && claude_composer_visible(screen))
    {
        // The turn ran and ended, with no limit line above.
        return Step::Done(State::Ready, None);
    }
    Step::Wait
}

#[allow(clippy::too_many_arguments)] // the probes and timings tests drive one by one
fn watch_prompted_with(
    clock: &dyn Clock,
    kind: AgentKind,
    mut capture: impl FnMut() -> Option<String>,
    mut alive: impl FnMut() -> bool,
    mut send_key: impl FnMut(&str) -> bool,
    bypass_requested: bool,
    fresh: bool,
    timing: Timing,
) -> Report {
    let started = clock.now();
    let mut answered: Vec<&'static str> = Vec::new();
    let mut last = String::new();
    let mut quiet_since = started;
    let mut working_since = None;
    let (state, reason) = loop {
        let now = clock.now();
        let screen = capture().map_or_else(String::new, |raw| {
            if kind == AgentKind::ClaudeCode {
                normalize_capture(raw)
            } else {
                raw
            }
        });
        if let Some(blocker) = launch_blocker(&screen).filter(|_| fresh) {
            break blocked_launch(blocker);
        }
        if let Some(reason) = io::blocked_screen(&screen) {
            break (State::Blocked, Some(reason.to_owned()));
        }
        let timed_out = now.saturating_duration_since(started) >= timing.timeout;
        if turn_running(&screen) {
            let since = *working_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= timing.working {
                break (State::Ready, None);
            }
            last.clear();
        } else {
            working_since = None;
            if screen != last {
                last = screen;
                quiet_since = now;
            } else if !last.trim().is_empty()
                && now.saturating_duration_since(quiet_since) >= timing.quiet
            {
                let step = settled_step(
                    kind,
                    &last,
                    &answered,
                    bypass_requested,
                    !timed_out,
                    &mut alive,
                );
                match step {
                    Step::Done(state, reason) => break (state, reason),
                    Step::Answer(dialog) => {
                        let typed = dialog
                            .keys_on(&last)
                            .is_some_and(|keys| type_keys(clock, &mut send_key, &keys));
                        if !typed {
                            break (
                                State::Blocked,
                                Some(format!("a startup dialog is waiting ({})", dialog.label)),
                            );
                        }
                        answered.push(dialog.label);
                        last.clear();
                        clock.sleep(dialog.pause.min(timing.timeout / 8));
                        continue;
                    }
                    Step::Wait => {}
                }
            }
        }
        if timed_out {
            break if let Some(line) = io::open_question(&last) {
                (
                    State::Blocked,
                    Some(format!("a dialog is waiting for an answer ({line})")),
                )
            } else if alive() {
                (
                    State::Unsettled,
                    Some(
                        "the agent did not start working on its prompt before the deadline".into(),
                    ),
                )
            } else {
                (State::Exited, Some(EXITED.into()))
            };
        }
        clock.sleep(timing.tick);
    };
    Report {
        state,
        answered: answered.into_iter().map(str::to_owned).collect(),
        reason,
    }
}

/// Whether a launch tend just made may answer the startup dialog `screen`
/// ends in: only initial setup (Claude's folder trust, Codex's update
/// menu), and bypass-permissions consent only when the launch asked for
/// that mode itself. Never an old goal, a task's approval, a folder's
/// settings or Codex's folder trust.
fn may_answer(kind: AgentKind, screen: &str, bypass_requested: bool) -> bool {
    pending_dialog(screen, STARTUP_DIALOGS, &[]).is_some_and(|d| {
        (d.label == "trusted the folder"
            // Codex trust can alter sandbox defaults; keep it blocked
            // until its cold trust flow is independently verified.
            && kind == AgentKind::ClaudeCode
            && d.pick == Some("Yes, I trust this folder")
            && option_rows(screen).is_some_and(|(rows, _)| {
                rows.len() == 2
                    && rows.iter().any(|row| {
                        matches!(row.as_str(), "No, exit" | "No, exit Claude Code" | "Quit")
                    })
            }))
            || (kind == AgentKind::Codex && d.label == "skipped the Codex update prompt")
            || (bypass_requested && d.consent == Consent::Bypass)
    })
}

/// The report for a launch its own screen says cannot work.
fn blocked_launch(blocker: LaunchBlocker) -> (State, Option<String>) {
    match blocker {
        LaunchBlocker::Limit(line) => (
            State::Limited,
            Some(format!("the account is at its usage limit (\"{line}\")")),
        ),
        LaunchBlocker::SignIn(reason) => (State::Blocked, Some(reason.to_owned())),
    }
}

/// Codex's startup review of new or changed hooks, as the live menu at the
/// bottom of the screen: its title, the numbered options with one selected,
/// and the key-hint footer last. Text that merely mentions it (a transcript
/// line above a normal composer) is not the menu.
#[must_use]
pub fn codex_hook_review(screen: &str) -> bool {
    let tail = crate::agent_tui::screen_tail(screen, 20);
    let lines: Vec<&str> = tail
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines
        .iter()
        .any(|line| line.starts_with('›') && is_codex_composer_line(line))
    {
        return false;
    }
    let Some(title) = lines.iter().rposition(|line| *line == "Hooks need review") else {
        return false;
    };
    // The footer is the last line or two; Codex wraps it in a narrow pane,
    // and older builds word it "Press enter to confirm or esc to go back".
    let footer_at = lines.len().saturating_sub(2).max(title + 1);
    let footer = lines[footer_at..].join(" ").to_lowercase();
    let footer_lines = if footer.contains("enter") && footer.contains("confirm") {
        lines[footer_at..]
            .iter()
            .position(|line| line.to_lowercase().contains("enter"))
            .map_or(0, |start| lines.len() - footer_at - start)
    } else {
        return false;
    };
    // Numbered rows; a wrapped label continues on the next lines.
    let mut rows: Vec<(bool, String)> = Vec::new();
    for line in &lines[title + 1..lines.len() - footer_lines] {
        let selected = line.starts_with('›');
        let body = line.trim_start_matches('›').trim_start();
        match body.split_once(". ") {
            Some((n, label)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                rows.push((selected, label.to_owned()));
            }
            _ => {
                if let Some((_, label)) = rows.last_mut() {
                    label.push(' ');
                    label.push_str(body);
                }
            }
        }
    }
    let position = |label: &str| rows.iter().position(|(_, row)| row == label);
    matches!(
        (position("Review hooks"), position("Trust all and continue")),
        (Some(review), Some(trust)) if review < trust
    ) && rows.iter().any(|(selected, _)| *selected)
}

/// An empty Codex composer or one showing a placeholder.
fn is_codex_composer_line(line: &str) -> bool {
    let text = line.trim_start_matches('›').trim();
    text.is_empty() || crate::agent_tui::CODEX_PLACEHOLDERS.contains(&text)
}

/// A known Codex placeholder cut at a word boundary by a narrow pane
/// (`› Ask Codex to`). Only for a worker launched a moment ago, which
/// nobody has typed into; account-swap draft capture keeps such text.
fn cut_placeholder(text: &str) -> bool {
    let text = text.trim_end_matches('…');
    text.chars().count() >= 8
        && crate::agent_tui::CODEX_PLACEHOLDERS
            .iter()
            .any(|placeholder| {
                placeholder
                    .strip_prefix(text)
                    .is_some_and(|rest| rest.starts_with(' '))
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tui::{DOWN, ENTER, ESC};
    use std::cell::Cell;
    use std::time::Instant;
    struct FakeClock {
        base: Instant,
        elapsed: Cell<Duration>,
    }
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + self.elapsed.get()
        }
        fn sleep(&self, d: Duration) {
            self.elapsed.set(self.elapsed.get() + d);
        }
    }
    #[test]
    fn codex_folder_trust_remains_blocked_without_a_verified_sandbox_flow() {
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let report = prepare_with(
            &clock,
            AgentKind::Codex,
            || {
                Some("Trust this folder\n› 1. Trust and continue\n  2. Quit\n  enter continue · esc quit".into())
            },
            || true,
            |_| panic!("must not accept Codex trust"),
            false,
            true,
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(25),
        );
        assert_ne!(report.state, State::Ready);
        assert!(report.answered.is_empty());
    }
    const TRUST: &str =
        " ❯ No, exit\n   Yes, I trust this folder\n Enter to confirm · Esc to cancel";
    const READY: &str = "Claude Code\n❯ \n ? for shortcuts";
    fn run(
        screen: impl FnMut() -> Option<String>,
        alive: impl FnMut() -> bool,
        send: impl FnMut(&str) -> bool,
    ) -> Report {
        prepare_with(
            &FakeClock {
                base: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
            },
            AgentKind::ClaudeCode,
            screen,
            alive,
            send,
            false,
            true,
            Duration::from_secs(3),
            Duration::from_millis(100),
            Duration::from_millis(25),
        )
    }
    #[test]
    fn custom_claude_statusline_does_not_hide_its_framed_composer() {
        let screen = "Claude Code v2.1.322\n─────────────────\n❯ \u{a0}\n─────────────────\n ~/p/tend Opus 5.5 0 0\n ⎇ main 453ms $0.00";
        assert_eq!(
            run(|| Some(screen.into()), || true, |_| panic!()).state,
            State::Ready
        );
        for unproven in ["Claude Code\n❯ ", "Claude Code\n─────\nLoading\n─────"]
        {
            assert_eq!(
                run(|| Some(unproven.into()), || true, |_| panic!()).state,
                State::Unsettled
            );
        }
        for suffix in [
            "\n$ claude --resume old",
            "\n❯ exec claude",
            "\nuser@host % claude --session-id new",
            "\nline1\nline2\nline3\nline4",
        ] {
            assert_eq!(
                run(|| Some(format!("{screen}{suffix}")), || true, |_| panic!()).state,
                State::Unsettled
            );
        }
        // The shared account-swap probe must retain its old policy: a dead
        // composer's frame can remain above a fresh relaunch.
        assert!(!crate::agent_tui::agent_prompt_visible(screen));
        let without_header = screen.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert_eq!(
            run(|| Some(without_header.clone()), || true, |_| panic!()).state,
            State::Ready
        );
        let draft = screen.replace("❯ \u{a0}", "❯ unsent draft");
        assert_eq!(
            run(|| Some(draft.clone()), || true, |_| panic!()).state,
            State::Blocked
        );
    }
    #[test]
    fn trust_is_answered_then_a_real_prompt_is_required() {
        let keys = std::cell::RefCell::new(Vec::new());
        let result = run(
            || {
                Some(
                    if keys.borrow().len() == 2 {
                        READY
                    } else {
                        TRUST
                    }
                    .into(),
                )
            },
            || true,
            |key| {
                keys.borrow_mut().push(key.to_owned());
                true
            },
        );
        assert_eq!(result.state, State::Ready);
        assert_eq!(*keys.borrow(), [DOWN, ENTER]);
        assert_eq!(result.answered, ["trusted the folder"]);
    }
    #[test]
    fn settings_tools_and_bypass_are_not_startup_authority() {
        for screen in [
            " ❯ No, exit\n   Yes, I trust these settings\n Enter to confirm · Esc to cancel",
            "Bypass Permissions mode\n ❯ No, exit\n   Yes, I accept\n Enter to confirm · Esc to cancel",
            "Allow this read outside the working directories?\n ❯ 1. Yes, always\n   2. No\n Enter to confirm · Esc to cancel",
        ] {
            let result = run(|| Some(screen.into()), || true, |_| panic!("must not answer a permission dialog"));
            assert_eq!(result.state, State::Blocked, "{screen}");
        }
    }
    #[test]
    fn process_exit_and_shell_echo_are_never_ready() {
        assert_eq!(
            run(|| Some(TRUST.into()), || false, |_| panic!()).state,
            State::Exited
        );
        assert_eq!(
            run(
                || Some("$ claude --session-id new".into()),
                || true,
                |_| panic!()
            )
            .state,
            State::Unsettled
        );
    }
    #[test]
    fn a_failed_write_does_not_repeat_an_answer() {
        let count = Cell::new(0);
        let result = run(
            || Some(TRUST.into()),
            || true,
            |_| {
                count.set(count.get() + 1);
                false
            },
        );
        assert_eq!(result.state, State::Blocked);
        assert_eq!(count.get(), 1);
    }
    #[test]
    fn a_lingering_gate_is_not_answered_twice() {
        let count = Cell::new(0);
        let result = run(
            || Some(TRUST.into()),
            || true,
            |_| {
                count.set(count.get() + 1);
                true
            },
        );
        assert_ne!(result.state, State::Ready);
        assert_eq!(
            count.get(),
            2,
            "only Down and Enter, never a repeated confirmation"
        );
    }
    #[test]
    fn a_draft_or_running_turn_is_not_an_empty_composer() {
        for screen in [
            "──────\n❯ existing draft\n──────\n? for shortcuts",
            "✻ Thinking… (12s · ↓ 300 tokens)\n──────\n❯ \n? for shortcuts",
        ] {
            let result = run(
                || Some(screen.into()),
                || true,
                |_| panic!("must not clear another input"),
            );
            assert_eq!(result.state, State::Blocked);
        }
    }
    #[test]
    fn codex_hook_review_is_a_named_blocker_and_never_answered() {
        let screen = "  Hooks need review\n  6 hooks are new or changed.\n  Hooks can run outside the\n  sandbox after you trust\n  them.\n› 1. Review hooks\n  2. Trust all and continue\n  3. Continue without trusting\n     (hooks won't run)\n  enter confirm · esc skip";
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let result = prepare_with(
            &clock,
            AgentKind::Codex,
            || Some(screen.into()),
            || true,
            |_| panic!("must not answer a hook review"),
            true,
            true,
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(25),
        );
        assert_eq!(result.state, State::Blocked);
        assert!(
            result
                .reason
                .as_deref()
                .unwrap()
                .contains("review new or changed hooks"),
            "{result:?}"
        );
        assert!(result.answered.is_empty());
    }

    #[test]
    fn a_narrow_codex_placeholder_is_not_a_draft_for_a_fresh_worker_only() {
        assert!(cut_placeholder("Ask Codex to"));
        assert!(cut_placeholder("Find and fix…"));
        assert!(!cut_placeholder("Ask Codex to fix it"));
        assert!(!cut_placeholder("Ask"));
        // Account-swap draft capture keeps such text: it may be typed.
        assert_eq!(
            crate::agent_tui::composer_draft(
                "› Find and fix\n  100% context left",
                AgentKind::Codex
            )
            .as_deref(),
            Some("Find and fix")
        );
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let result = prepare_with(
            &clock,
            AgentKind::Codex,
            || Some("  permissions: YOLO mode\n› Ask Codex to\n  G…  ⚠ 1 · f2".into()),
            || true,
            |_| true,
            false,
            true,
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(25),
        );
        assert_eq!(result.state, State::Ready, "{result:?}");
    }

    #[test]
    fn a_wrapped_hook_menu_and_older_footer_are_still_the_menu() {
        let wrapped = "  Hooks need review\n  6 hooks are new or changed.\n› 1. Review hooks\n  2. Trust all and\n     continue\n  3. Continue without\n     trusting (hooks won't run)\n  enter confirm ·\n  esc skip";
        assert!(codex_hook_review(wrapped));
        let older = "  Hooks need review\n› 1. Review hooks\n  2. Trust all and continue\n  3. Continue without trusting\n  Press enter to confirm or esc to go back";
        assert!(codex_hook_review(older));
        let unselected = "  Hooks need review\n  1. Review hooks\n  2. Trust all and continue\n  enter confirm · esc skip";
        assert!(!codex_hook_review(unselected));
    }

    #[test]
    fn a_transcript_mentioning_the_hook_review_is_not_the_menu() {
        let idle = "• The startup dialog says Hooks need review. Choose Review hooks to inspect it.\n› Ask Codex to do anything\n  100% context left";
        assert!(!codex_hook_review(idle));
        let quoted = "  Hooks need review\n  1. Review hooks\n  2. Trust all and continue\n› Ask Codex to do anything\n  100% context left";
        assert!(!codex_hook_review(quoted));
    }

    #[test]
    fn narrow_claude_hard_wrapping_is_handled_without_guessing_other_dialogs() {
        let count = Cell::new(0);
        let narrow = "Security guide\n\n ❯ No, exit\n   Yes, I trust this\n   folder\n\n Enter to confirm ·\n Esc to cancel\n";
        let result = run(
            || Some(if count.get() == 2 { READY } else { narrow }.into()),
            || true,
            |_| {
                count.set(count.get() + 1);
                true
            },
        );
        assert_eq!(result.state, State::Ready);
        assert_eq!(count.get(), 2);
    }

    // A fake agent launched with its first prompt: what it draws depends
    // on the time since launch and the keys typed so far.
    const WORKING: &str = "> fix the build\n\n✻ Thinking… (3s · ↓ 120 tokens)\n\n─────\n❯ \n─────\n  ⏵⏵ bypass permissions on · esc to interrupt";
    const ANSWERED: &str = "> fix the build\n\n● Done: the build is fixed.\n\n─────\n❯ \u{a0}\n─────\n  ⏵⏵ bypass permissions on (shift+tab to cycle)";
    // jiun-mini, 2026-11-22: a narrow pane's weekly limit, as captured.
    const WEEKLY_LIMIT: &str = "> review this\n  ⎿  You've hit your\n     weekly limit ·\n     resets Nov 25 at\n     10pm\n     (Asia/Seoul)\n     /upgrade or\n     /usage-credits\n     to finish what\n     you\u{2019}re working\n     on.\n❯ \n  ⏵⏵ auto mode on";
    const CODEX_LIMIT: &str = "› fix the build\n\n■ You've hit your usage limit. Upgrade to Pro (https://openai.com/chatgpt/pricing) or try again at 3:05 PM.\n\n› Ask Codex to do anything\n  ⏎ send   ⌃J newline";
    const LOGIN: &str = " Claude Code can be used with your Claude subscription or billed based on API usage.\n\n Select login method:\n\n ❯ 1. Claude account with subscription · Pro, Max, Team, or Enterprise\n   2. Anthropic Console account · API usage billing\n   3. 3rd-party platform · Amazon Bedrock, Microsoft Foundry, or Vertex AI";
    const CODEX_UPDATE: &str = "  ✨ Update available! 0.170.0 -> 0.171.0\n\n› 1. Update now (runs `brew upgrade codex`)\n  2. Skip\n  3. Skip until next version\n\n  Press enter to continue";
    const SETTINGS_TRUST: &str = " This folder's settings allow tools and hooks.\n ❯ 1. Yes, I trust these settings\n   2. No, exit\n Enter to confirm · Esc to cancel";
    const BYPASS: &str = " WARNING: Claude Code running in Bypass Permissions mode\n ❯ 1. No, exit\n   2. Yes, I accept\n Enter to confirm · Esc to cancel";

    fn watch(
        kind: AgentKind,
        bypass: bool,
        mut screen: impl FnMut(Duration, &[String]) -> Option<String>,
    ) -> (Report, Vec<String>) {
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let keys = std::cell::RefCell::new(Vec::new());
        let report = watch_prompted_with(
            &clock,
            kind,
            || screen(clock.elapsed.get(), &keys.borrow()),
            || true,
            |key| {
                keys.borrow_mut().push(key.to_owned());
                true
            },
            bypass,
            true,
            Timing {
                timeout: Duration::from_secs(20),
                quiet: Duration::from_millis(200),
                working: Duration::from_secs(2),
                tick: Duration::from_millis(50),
            },
        );
        (report, keys.into_inner())
    }

    #[test]
    fn a_prompted_launch_is_started_once_its_turn_has_run_a_while() {
        let (report, keys) = watch(AgentKind::ClaudeCode, true, |_, _| Some(WORKING.into()));
        assert_eq!(report.state, State::Ready, "{report:?}");
        assert!(keys.is_empty());
        // A turn that ends at once is started too.
        let (report, _) = watch(AgentKind::ClaudeCode, true, |_, _| Some(ANSWERED.into()));
        assert_eq!(report.state, State::Ready, "{report:?}");
    }

    #[test]
    fn a_prompted_launch_answers_folder_trust_and_its_prompt_runs_after() {
        let (report, keys) = watch(AgentKind::ClaudeCode, true, |_, keys| {
            Some(if keys.len() < 2 { TRUST } else { WORKING }.into())
        });
        assert_eq!(report.state, State::Ready, "{report:?}");
        assert_eq!(keys, [DOWN, ENTER]);
        assert_eq!(report.answered, ["trusted the folder"]);
    }

    #[test]
    fn a_limit_after_the_spinner_is_a_capped_launch_not_a_started_one() {
        // The spinner shows for a second, then the limit answer.
        let (report, keys) = watch(AgentKind::ClaudeCode, true, |at, _| {
            Some(
                if at < Duration::from_secs(1) {
                    WORKING
                } else {
                    WEEKLY_LIMIT
                }
                .into(),
            )
        });
        assert_eq!(report.state, State::Limited, "{report:?}");
        let reason = report.reason.unwrap();
        assert!(
            reason.contains("You've hit your weekly limit · resets Nov 25 at 10pm (Asia/Seoul)"),
            "{reason}"
        );
        assert!(keys.is_empty());
        let (report, _) = watch(AgentKind::Codex, true, |_, _| Some(CODEX_LIMIT.into()));
        assert_eq!(report.state, State::Limited, "{report:?}");
        assert!(report
            .reason
            .unwrap()
            .contains("usage limit. Upgrade to Pro"));
    }

    #[test]
    fn a_resumed_conversation_is_never_read_for_a_limit() {
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        // Its history ends in the limit it was capped on before the swap.
        let report = watch_prompted_with(
            &clock,
            AgentKind::ClaudeCode,
            || Some(WEEKLY_LIMIT.into()),
            || true,
            |_| panic!(),
            true,
            false,
            Timing {
                timeout: Duration::from_secs(5),
                quiet: Duration::from_millis(200),
                working: Duration::from_secs(2),
                tick: Duration::from_millis(50),
            },
        );
        assert_eq!(report.state, State::Ready, "{report:?}");
    }

    #[test]
    fn sign_in_and_consent_screens_stop_the_launch_with_nothing_typed() {
        let typed_nothing = |kind, bypass, screen: &'static str| {
            let (report, keys) = watch(kind, bypass, |_, _| Some(screen.into()));
            assert_eq!(report.state, State::Blocked, "{screen}: {report:?}");
            assert!(keys.is_empty(), "{screen}: typed {keys:?}");
            report.reason.unwrap()
        };
        assert!(typed_nothing(AgentKind::ClaudeCode, true, LOGIN).contains("not signed in"));
        assert!(typed_nothing(AgentKind::ClaudeCode, true, SETTINGS_TRUST)
            .contains("trust the folder's settings"));
        // Bypass consent is given only for a launch that asked for the mode.
        assert!(typed_nothing(AgentKind::ClaudeCode, false, BYPASS)
            .contains("accepted bypass-permissions mode"));
        let hooks = "  Hooks need review\n  6 hooks are new or changed.\n› 1. Review hooks\n  2. Trust all and continue\n  3. Continue without trusting\n  enter confirm · esc skip";
        assert!(typed_nothing(AgentKind::Codex, true, hooks).contains("hooks"));
        let codex_trust =
            "Trust this folder\n› 1. Trust and continue\n  2. Quit\n  enter continue · esc quit";
        typed_nothing(AgentKind::Codex, true, codex_trust);
    }

    #[test]
    fn safe_startup_menus_are_answered_once_then_the_prompt_runs() {
        let (report, keys) = watch(AgentKind::ClaudeCode, true, |_, keys| {
            Some(if keys.len() < 2 { BYPASS } else { WORKING }.into())
        });
        assert_eq!(report.state, State::Ready, "{report:?}");
        assert_eq!(keys, [DOWN, ENTER]);
        assert_eq!(report.answered, ["accepted bypass-permissions mode"]);
        let codex_working = "› fix the build\n\n• Working (3s • esc to interrupt)";
        let (report, keys) = watch(AgentKind::Codex, false, |_, keys| {
            Some(
                if keys.is_empty() {
                    CODEX_UPDATE
                } else {
                    codex_working
                }
                .into(),
            )
        });
        assert_eq!(report.state, State::Ready, "{report:?}");
        assert_eq!(keys, [ESC]);
        assert_eq!(report.answered, ["skipped the Codex update prompt"]);
    }

    #[test]
    fn a_launch_that_exits_or_never_draws_is_not_started() {
        let clock = FakeClock {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let timing = Timing {
            timeout: Duration::from_secs(5),
            quiet: Duration::from_millis(200),
            working: Duration::from_secs(2),
            tick: Duration::from_millis(50),
        };
        let gone = watch_prompted_with(
            &clock,
            AgentKind::ClaudeCode,
            || Some("$ ".into()),
            || false,
            |_| panic!(),
            true,
            true,
            timing,
        );
        assert_eq!(gone.state, State::Exited, "{gone:?}");
        let slow = watch_prompted_with(
            &clock,
            AgentKind::ClaudeCode,
            || Some("Loading…".into()),
            || true,
            |_| panic!(),
            true,
            true,
            timing,
        );
        assert_eq!(slow.state, State::Unsettled, "{slow:?}");
    }

    #[test]
    fn a_worker_launch_on_a_signed_out_account_names_the_sign_in() {
        let report = run(
            || Some(LOGIN.into()),
            || true,
            |_| panic!("typed into login"),
        );
        assert_eq!(report.state, State::Blocked);
        assert!(report.reason.unwrap().contains("not signed in"));
    }
}
