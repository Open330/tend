//! What Tend calls the tmux sessions, windows and panes it creates.
//!
//! `tend init` turns `automatic-rename` off, so a window keeps whatever name
//! it had when it was made. A window Tend creates without `-n` is therefore
//! called `zsh` or `aas` for the rest of its life — the command that happened
//! to start it. Every create path names what it holds instead:
//!
//! - a **session** after the project: the git checkout's directory, else the
//!   working directory;
//! - a **window** after the Work id, else the task, the alias, the role, and
//!   last the agent program;
//! - a **pane** title after the agent's alias, else its role.
//!
//! Names are set only when Tend creates the thing. Nothing here renames a
//! window or session that already exists: a name someone typed by hand is
//! theirs, and a mass rename of the owner's layout is not a naming fix.

use std::path::Path;

/// The longest name derived from free text (a task title). Work ids and
/// aliases are already short; a sentence-long task is not, and the status
/// line has room for a handful of windows.
pub const DERIVED_NAME_MAX_CHARS: usize = 24;

/// A short, slug-safe name for free text, or `None` when nothing nameable is
/// left.
///
/// Letters and digits survive in any script (a Korean task stays readable);
/// everything else becomes one `-`. Lowercased, so `Fix Login` and
/// `fix login` name the same window. Cut at the last `-` before `max_chars`
/// when there is one, so a title is not cut mid-word.
#[must_use]
pub fn slug(raw: &str, max_chars: usize) -> Option<String> {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        return None;
    }
    if out.chars().count() <= max_chars {
        return Some(out.to_string());
    }
    let cut: String = out.chars().take(max_chars).collect();
    if out.chars().nth(max_chars) == Some('-') {
        return Some(cut);
    }
    let cut = match cut.rfind('-') {
        // Keep at least half the budget; a cut that leaves one short word is
        // worse than a word cut in half.
        Some(at) if cut[..at].chars().count() >= max_chars / 2 => cut[..at].to_string(),
        _ => cut,
    };
    Some(cut.trim_end_matches('-').to_string())
}

/// The window name for one agent Tend starts outside any Work: its task,
/// else its alias, else its role, else the agent program (`claude`).
#[must_use]
pub fn agent_window_base(
    task: Option<&str>,
    alias: Option<&str>,
    role: Option<&str>,
    agent: &str,
) -> String {
    [task, alias, role]
        .into_iter()
        .flatten()
        .find_map(|text| slug(text, DERIVED_NAME_MAX_CHARS))
        .or_else(|| slug(agent, DERIVED_NAME_MAX_CHARS))
        .unwrap_or_else(|| "agent".into())
}

/// The pane title for an agent Tend starts: its alias, else its role. `None`
/// leaves the title alone — tmux's default (the host name) is better than a
/// made-up one, and the agent may set its own.
#[must_use]
pub fn agent_pane_title(alias: Option<&str>, role: Option<&str>) -> Option<String> {
    [alias, role]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|text| !text.is_empty() && !text.chars().any(char::is_control))
        .map(|text| text.chars().take(64).collect())
}

/// Whether `list-windows -F '#{window_id}\t#{window_name}'` output already
/// gives `name` to a window other than `window_id`.
///
/// Equality on the id, not the name, is what lets a window keep the name it
/// already has: re-submitting an unchanged prefill must be a no-op, not a
/// collision with itself. Pass an empty `window_id` for a window that does
/// not exist yet.
#[must_use]
pub fn window_name_taken(listing: &str, window_id: &str, name: &str) -> bool {
    listing.lines().any(|line| {
        line.split_once('\t')
            .is_some_and(|(id, current)| id.trim() != window_id && current.trim() == name)
    })
}

/// `base`, or `base-2`, `base-3`… — the first one no window in `listing`
/// uses.
#[must_use]
pub fn unique_window_name(listing: &str, base: &str) -> String {
    if !window_name_taken(listing, "", base) {
        return base.to_string();
    }
    (2..10_000)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !window_name_taken(listing, "", candidate))
        .unwrap_or_else(|| format!("{base}-overflow"))
}

/// The `list-windows` format [`window_name_taken`] reads.
pub const WINDOW_NAMES_FORMAT: &str = "#{window_id}\t#{window_name}";

/// The windows of one session on the ambient server, in
/// [`WINDOW_NAMES_FORMAT`]. Empty when tmux cannot say: a name that might
/// repeat one is a smaller fault than a launch that fails over a listing.
#[must_use]
pub fn session_window_listing(session: &str) -> String {
    let Ok(mut command) = crate::mux_control::ambient_command_with_args(&[
        "list-windows",
        "-t",
        session,
        "-F",
        WINDOW_NAMES_FORMAT,
    ]) else {
        return String::new();
    };
    crate::subprocess::output(&mut command)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

/// Title the pane Tend just created. Best effort: the pane already runs, and
/// a server that cannot title it (an older rmux) has nothing to undo.
///
/// `select-pane -T` only sets the title; it does not make the pane active.
pub fn title_pane(pane: &str, title: &str) {
    if let Ok(mut command) =
        crate::mux_control::ambient_command_with_args(&["select-pane", "-t", pane, "-T", title])
    {
        let _ = crate::subprocess::output(&mut command);
    }
}

/// The project a directory belongs to: the git checkout's top directory
/// when there is one, else the directory itself. Starting in `repo/crates/x`
/// names the session `repo`, not `x`. A linked worktree is its own checkout
/// and keeps its own directory name, which is what tells it apart from the
/// main one.
#[must_use]
pub fn project_dir(cwd: &Path) -> std::path::PathBuf {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"]);
    crate::subprocess::output(&mut command)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|top| !top.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|top| top.is_dir())
        .unwrap_or_else(|| cwd.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_title_becomes_a_short_slug() {
        assert_eq!(
            slug("Fix the login redirect", 24).as_deref(),
            Some("fix-the-login-redirect")
        );
        assert_eq!(
            slug("  Review PR #696: fleet keep/close  ", 24).as_deref(),
            Some("review-pr-696-fleet-keep")
        );
        assert_eq!(slug("로그인 고치기", 24).as_deref(), Some("로그인-고치기"));
        assert_eq!(slug("!!!", 24), None);
        assert_eq!(slug("", 24), None);
        assert_eq!(
            slug("supercalifragilisticexpialidocious", 24).as_deref(),
            Some("supercalifragilisticexpi")
        );
    }

    #[test]
    fn a_window_is_named_by_task_then_alias_then_role_then_program() {
        assert_eq!(
            agent_window_base(Some("Ship v2"), Some("rev"), Some("reviewer"), "claude"),
            "ship-v2"
        );
        assert_eq!(
            agent_window_base(Some("  "), Some("Rev"), Some("reviewer"), "claude"),
            "rev"
        );
        assert_eq!(
            agent_window_base(None, None, Some("reviewer"), "codex"),
            "reviewer"
        );
        assert_eq!(agent_window_base(None, None, None, "claude"), "claude");
    }

    #[test]
    fn a_pane_title_is_the_alias_else_the_role() {
        assert_eq!(
            agent_pane_title(Some("lead"), Some("reviewer")).as_deref(),
            Some("lead")
        );
        assert_eq!(
            agent_pane_title(Some(" "), Some("reviewer")).as_deref(),
            Some("reviewer")
        );
        assert_eq!(agent_pane_title(None, None), None);
    }

    #[test]
    fn a_derived_window_name_never_repeats_one_in_the_session() {
        let listing = "@1\tclaude\n@2\tclaude-2\n@3\tship-v2\n";
        assert_eq!(unique_window_name(listing, "claude"), "claude-3");
        assert_eq!(unique_window_name(listing, "codex"), "codex");
        assert_eq!(unique_window_name("", "claude"), "claude");
    }

    #[test]
    fn window_name_taken_excludes_the_window_being_renamed() {
        let listing = "@1\tmain\n@4\teng-7306\n@5\treview\n";
        assert!(!window_name_taken(listing, "@4", "eng-7306"));
        assert!(window_name_taken(listing, "@4", "review"));
        assert!(window_name_taken(listing, "", "eng-7306"));
    }

    #[test]
    fn a_subdirectory_names_its_checkout() {
        let repo = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .is_ok_and(|status| status.success());
        if !ok {
            return; // no git on this runner; the fallback is covered below
        }
        let sub = repo.path().join("crates").join("x");
        std::fs::create_dir_all(&sub).unwrap();
        let top = tend::paths::canonicalize(repo.path()).unwrap();
        let sub = tend::paths::canonicalize(&sub).unwrap();
        assert_eq!(tend::paths::canonicalize(project_dir(&sub)).unwrap(), top);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(project_dir(plain.path()), plain.path());
    }
}
