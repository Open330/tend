//! Start a new agent on a chosen account.
//!
//! Without this, a tend launch runs the bare provider CLI on whichever
//! account it happens to be logged into, even while that account is capped,
//! so every new worker starts already stopped. `[agent.<program>].account`
//! lets tend choose instead: `auto` takes the configured account with the
//! most quota tend has seen left (or `aas`'s best-ranked one with quota
//! left), a name pins one of that provider's accounts.
//!
//! The accounts come from [`crate::account_provider`]: configured
//! `[[accounts.<provider>]]`, ranked by observed usage, else `aas` when it is
//! installed. Neither is required. When there is no backend, or it is
//! slow, the agent starts bare exactly as it would without the setting.
//! When every account is at its usage limit, or the pinned one is, the
//! agent does not start ([`NotRouted::Capped`]): it would only sit on the
//! limit screen. The decision is made before any pane exists, so neither a
//! failed query nor a refusal leaves half-built layout behind.
//!
//! Already-running agents are [`crate::account_swap`]'s business.

use crate::account_swap::{find_aas, query_candidates_within, Candidate};
use std::time::Duration;
pub use tend::swap_api::{
    report, AccountDecision, AccountRoute, NotRouted, RoutedLaunch, SpawnAccount, PROVIDERS,
};

/// How long `aas candidates` gets before a launch gives up on it. `aas`
/// answers from its usage cache, so this only trips when it hangs.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// A configured account, by name or, for `auto`, the first eligible one in
/// [`crate::account_provider::configured`]'s ranking. When every account is
/// held capped it is the first in config order, as before ranking: a swap
/// moves the agent on when it is capped.
fn from_profiles(route: &AccountRoute, passed_over: &[String]) -> Result<RoutedLaunch, NotRouted> {
    from_profiles_with(
        route,
        passed_over,
        &crate::account_usage::load_default(),
        time::OffsetDateTime::now_utc(),
    )
}

fn from_profiles_with(
    route: &AccountRoute,
    passed_over: &[String],
    usage: &crate::account_usage::Ledger,
    now: time::OffsetDateTime,
) -> Result<RoutedLaunch, NotRouted> {
    let provider = route.provider.as_str();
    if cfg!(windows) {
        return Err(NotRouted::Bare(
            "configured accounts launch through `env`/`sh`, which this shell lacks".into(),
        ));
    }
    let capped: Vec<&str> = passed_over.iter().map(String::as_str).collect();
    let ranked =
        crate::account_provider::configured(provider, &route.profiles, &capped, usage, now);
    let profile = match &route.choice {
        SpawnAccount::Named(name) => {
            let profile = tend::accounts::find_profile(&route.profiles, provider, name)
                .ok_or_else(|| {
                    NotRouted::Bare(format!(
                        "no {provider} account named {name} in [[accounts.{provider}]]"
                    ))
                })?;
            if let Some(held) = ranked
                .iter()
                .find(|c| c.name == profile.name && !c.eligible)
            {
                return Err(NotRouted::Capped(pinned_capped(
                    &held.name,
                    &held.reason,
                    provider,
                )));
            }
            profile
        }
        SpawnAccount::Auto => {
            if route.profiles.is_empty() {
                return Err(NotRouted::Bare(format!(
                    "[[accounts.{provider}]] lists no account"
                )));
            }
            ranked
                .iter()
                .find(|c| c.eligible)
                .and_then(|c| route.profiles.iter().find(|p| p.name == c.name))
                .ok_or_else(|| NotRouted::Capped(all_capped(&ranked, provider)))?
        }
    };
    Ok(RoutedLaunch {
        account: profile.name.clone(),
        launcher: profile.launch_prefix(provider),
    })
}

/// Why a pinned account does not start.
fn pinned_capped(name: &str, reason: &str, provider: &str) -> String {
    format!(
        "{name} is at its usage limit ({reason}); it is pinned as [agent.{provider}].account, \
         so tend did not start on another one: wait for its reset, or set the account to \
         \"auto\""
    )
}

/// Why no `provider` account starts, naming each one passed over.
fn all_capped(ranked: &[Candidate], provider: &str) -> String {
    let passed: Vec<String> = ranked
        .iter()
        .filter(|c| c.provider == provider)
        .map(|c| format!("{}: {}", c.name, c.reason))
        .collect();
    format!(
        "every {provider} account is at its usage limit ({})",
        passed.join(", ")
    )
}

/// Whether `aas` holds `candidate` back for its quota, rather than for a
/// reason tend cannot read (which starts the launch as before).
fn at_limit(candidate: &Candidate) -> bool {
    !candidate.eligible && {
        let reason = candidate.reason.to_ascii_lowercase();
        reason.contains("limit") || reason.contains("capped") || reason.contains("cooldown")
    }
}

/// The first eligible `provider` account, in the backend's ranking.
pub fn choose(candidates: &[Candidate], provider: &str) -> Result<String, String> {
    candidates
        .iter()
        .find(|c| c.eligible && c.provider == provider)
        .map(|c| c.name.clone())
        .ok_or_else(|| {
            let passed: Vec<String> = candidates
                .iter()
                .filter(|c| c.provider == provider)
                .map(|c| format!("{}: {}", c.name, c.reason))
                .collect();
            if passed.is_empty() {
                format!("no {provider} account is available")
            } else {
                format!(
                    "no {provider} account has quota left ({})",
                    passed.join(", ")
                )
            }
        })
}

/// A pinned account, but only for its own provider: `aas e` picks the CLI
/// from the account, so `work@claude` under `[agent.codex]` would start
/// Claude with Codex's arguments. One at its usage limit does not start:
/// the agent would only show the limit screen.
fn pinned(candidates: &[Candidate], name: &str, provider: &str) -> Result<String, NotRouted> {
    match candidates.iter().find(|c| c.name == name) {
        Some(c) if c.provider == provider && at_limit(c) => {
            Err(NotRouted::Capped(pinned_capped(name, &c.reason, provider)))
        }
        Some(c) if c.provider == provider => Ok(name.to_owned()),
        Some(c) => Err(NotRouted::Bare(format!(
            "{name} is a {} account, not {provider}",
            c.provider
        ))),
        None => Err(NotRouted::Bare(format!(
            "aas has no {provider} account named {name}"
        ))),
    }
}

/// Decide where a new agent starts. `passed_over` are accounts this launch
/// already found at their limit, which `auto` skips and a pin refuses.
pub fn decide(route: &AccountRoute, passed_over: &[String]) -> Result<RoutedLaunch, NotRouted> {
    if route.module_off {
        return Err(NotRouted::Bare(tend::swap_api::module_unavailable(true)));
    }
    let routed = decide_with(route, passed_over, |aas, provider| {
        query_candidates_within(aas, provider, QUERY_TIMEOUT)
    })?;
    // A Codex launch on an account home meets the same "Hooks need review"
    // menu a swap would: carry over trust the user gave elsewhere first.
    if route.provider == "codex" {
        if let Some(note) =
            crate::codex_hook_trust::prepare_launch(&route.profiles, &routed, &route.aas)
        {
            eprintln!("tend: {note}");
        }
    }
    Ok(routed)
}

fn decide_with(
    route: &AccountRoute,
    passed_over: &[String],
    query: impl FnOnce(&str, &str) -> Result<Vec<Candidate>, String>,
) -> Result<RoutedLaunch, NotRouted> {
    decide_found(route, passed_over, find_aas, query)
}

fn decide_found(
    route: &AccountRoute,
    passed_over: &[String],
    find: impl FnOnce(&str) -> Option<String>,
    query: impl FnOnce(&str, &str) -> Result<Vec<Candidate>, String>,
) -> Result<RoutedLaunch, NotRouted> {
    if !route.profiles.is_empty() {
        return from_profiles(route, passed_over);
    }
    let aas = find(&route.aas).ok_or_else(|| {
        NotRouted::Bare(
            if route.aas == crate::account_swap::SwapConfig::default().aas {
                crate::account_provider::no_accounts(&route.provider)
            } else {
                format!("`{}` not found", route.aas)
            },
        )
    })?;
    let mut candidates = query(&aas, &route.provider).map_err(NotRouted::Bare)?;
    // Seen at the limit by this launch, whatever aas's cache says.
    for candidate in &mut candidates {
        if passed_over.contains(&candidate.name) {
            candidate.eligible = false;
            candidate.reason = "hit its usage limit as this launch started".into();
        }
    }
    let account = match &route.choice {
        SpawnAccount::Named(name) => pinned(&candidates, name, &route.provider)?,
        SpawnAccount::Auto => choose(&candidates, &route.provider).map_err(|reason| {
            if candidates.iter().any(|c| c.provider == route.provider) {
                NotRouted::Capped(all_capped(&candidates, &route.provider))
            } else {
                NotRouted::Bare(reason)
            }
        })?,
    };
    Ok(RoutedLaunch::via_aas(aas, account))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, provider: &str, eligible: bool) -> Candidate {
        Candidate {
            name: name.into(),
            provider: provider.into(),
            eligible,
            active: false,
            reason: if eligible {
                "quota_available"
            } else {
                "usage_rate_limited"
            }
            .into(),
            remaining_pct: None,
        }
    }

    fn bare(not: NotRouted) -> String {
        match not {
            NotRouted::Bare(reason) => reason,
            NotRouted::Capped(reason) => panic!("expected a bare start, got capped: {reason}"),
        }
    }

    fn route(choice: SpawnAccount) -> AccountRoute {
        AccountRoute {
            provider: "claude".into(),
            choice,
            // Any file that exists stands in for the binary.
            aas: std::env::current_exe().unwrap().display().to_string(),
            profiles: Vec::new(),
            module_off: false,
        }
    }

    #[test]
    fn parse_reads_auto_names_and_blank() {
        assert_eq!(SpawnAccount::parse(" auto "), Some(SpawnAccount::Auto));
        assert_eq!(
            SpawnAccount::parse("spare@claude"),
            Some(SpawnAccount::Named("spare@claude".into()))
        );
        assert_eq!(SpawnAccount::parse("  "), None);
    }

    #[test]
    fn choose_keeps_aas_ranking_and_skips_capped_and_other_providers() {
        let list = [
            candidate("cx@codex", "codex", true),
            candidate("capped@claude", "claude", false),
            candidate("second@claude", "claude", true),
            candidate("third@claude", "claude", true),
        ];
        assert_eq!(choose(&list, "claude").unwrap(), "second@claude");
        assert_eq!(choose(&list, "codex").unwrap(), "cx@codex");
    }

    #[test]
    fn choose_names_every_passed_over_account() {
        let list = [
            candidate("a@claude", "claude", false),
            candidate("cx@codex", "codex", true),
        ];
        let reason = choose(&list, "claude").unwrap_err();
        assert!(reason.contains("a@claude: usage_rate_limited"), "{reason}");
        assert!(!reason.contains("cx@codex"), "{reason}");
        assert_eq!(
            choose(&[], "codex").unwrap_err(),
            "no codex account is available"
        );
    }

    #[test]
    fn auto_queries_the_resolved_binary_for_its_provider() {
        let route = route(SpawnAccount::Auto);
        let routed = decide_with(&route, &[], |aas, provider| {
            assert_eq!(aas, route.aas);
            assert_eq!(provider, "claude");
            Ok(vec![candidate("ok@claude", "claude", true)])
        })
        .unwrap();
        assert_eq!(routed.account, "ok@claude");
        assert_eq!(routed.launcher, [route.aas.as_str(), "e", "ok@claude"]);
    }

    #[test]
    fn a_named_account_starts_only_for_its_provider_and_not_at_its_limit() {
        let list = || {
            Ok(vec![
                candidate("pin@claude", "claude", true),
                candidate("capped@claude", "claude", false),
                candidate("cx@codex", "codex", true),
            ])
        };
        let named = |name: &str| route(SpawnAccount::Named(name.into()));
        let routed = decide_with(&named("pin@claude"), &[], |_, _| list()).unwrap();
        assert_eq!(routed.account, "pin@claude");
        // Pinned and capped: refused up front, never started on another.
        let Err(NotRouted::Capped(reason)) =
            decide_with(&named("capped@claude"), &[], |_, _| list())
        else {
            panic!("a capped pin must refuse");
        };
        assert!(
            reason.starts_with("capped@claude is at its usage limit (usage_rate_limited)"),
            "{reason}"
        );
        assert!(reason.contains("\"auto\""), "{reason}");
        // Seen at the limit by this launch, though aas still lists it eligible.
        assert!(matches!(
            decide_with(&named("pin@claude"), &["pin@claude".into()], |_, _| list()),
            Err(NotRouted::Capped(_))
        ));
        // An account aas holds back for another reason still starts bare.
        let odd = || {
            let mut other = candidate("odd@claude", "claude", false);
            other.reason = "token_unreadable".into();
            Ok(vec![other])
        };
        assert_eq!(
            decide_with(&named("odd@claude"), &[], |_, _| odd())
                .unwrap()
                .account,
            "odd@claude"
        );
        assert_eq!(
            decide_with(&named("cx@codex"), &[], |_, _| list()).unwrap_err(),
            NotRouted::Bare("cx@codex is a codex account, not claude".into())
        );
        assert_eq!(
            decide_with(&named("nope"), &[], |_, _| list()).unwrap_err(),
            NotRouted::Bare("aas has no claude account named nope".into())
        );
    }

    #[test]
    fn auto_skips_accounts_this_launch_saw_capped_and_refuses_when_none_is_left() {
        let list = || {
            Ok(vec![
                candidate("first@claude", "claude", true),
                candidate("second@claude", "claude", true),
                candidate("cx@codex", "codex", true),
            ])
        };
        let auto = route(SpawnAccount::Auto);
        let routed = decide_with(&auto, &["first@claude".into()], |_, _| list()).unwrap();
        assert_eq!(routed.account, "second@claude");
        let Err(NotRouted::Capped(reason)) = decide_with(
            &auto,
            &["first@claude".into(), "second@claude".into()],
            |_, _| list(),
        ) else {
            panic!("no account left must refuse");
        };
        assert!(
            reason.starts_with("every claude account is at its usage limit"),
            "{reason}"
        );
        assert!(
            reason.contains("second@claude: hit its usage limit"),
            "{reason}"
        );
        assert!(!reason.contains("cx@codex"), "{reason}");
        // No account of the provider at all is not a limit: it starts bare.
        assert!(matches!(
            decide_with(&auto, &[], |_, _| Ok(Vec::new())),
            Err(NotRouted::Bare(_))
        ));
    }

    #[test]
    fn a_missing_binary_or_failed_query_starts_bare_with_a_reason() {
        let mut missing = route(SpawnAccount::Named("pin@claude".into()));
        missing.aas = "/nonexistent/aas".into();
        let outcome = decide_with(&missing, &[], |_, _| unreachable!()).map_err(bare);
        assert_eq!(
            report(&outcome),
            AccountDecision {
                account: None,
                skipped: Some("`/nonexistent/aas` not found".into()),
                passed_over: Vec::new(),
            }
        );
        let outcome = decide_with(&route(SpawnAccount::Auto), &[], |_, _| {
            Err("timed out".into())
        })
        .map_err(bare);
        assert_eq!(report(&outcome).skipped.as_deref(), Some("timed out"));
    }

    #[test]
    fn no_accounts_and_no_aas_starts_bare_and_says_how_to_configure_one() {
        let mut bare = route(SpawnAccount::Auto);
        // The default `aas`, not installed: tend needs no external tool.
        bare.aas = crate::account_swap::SwapConfig::default().aas;
        let NotRouted::Bare(reason) =
            decide_found(&bare, &[], |_| None, |_, _| unreachable!()).unwrap_err()
        else {
            panic!("no backend starts bare");
        };
        assert!(reason.contains("[[accounts.claude]]"), "{reason}");
        assert!(reason.contains("resume-after-limit"), "{reason}");
    }

    #[test]
    fn argv_runs_the_provider_args_through_aas_unchanged() {
        let routed = RoutedLaunch::via_aas("/opt/aas".into(), "ok@codex".into());
        assert_eq!(
            routed.argv(["--yolo".into(), "say \"hi\"\nthere".into()]),
            ["/opt/aas", "e", "ok@codex", "--yolo", "say \"hi\"\nthere"]
        );
    }

    fn profile(name: &str) -> tend::accounts::AccountProfile {
        tend::accounts::AccountProfile {
            name: name.into(),
            env: [("CLAUDE_CONFIG_DIR".to_owned(), format!("/p/{name}"))].into(),
            env_file: None,
        }
    }

    #[test]
    fn configured_accounts_route_without_aas() {
        let mut route = route(SpawnAccount::Auto);
        route.aas = "/nonexistent/aas".into();
        route.profiles = vec![profile("work"), profile("home")];
        let never = |_: &str, _: &str| -> Result<Vec<Candidate>, String> {
            panic!("aas must not be queried")
        };
        let auto = decide_with(&route, &[], never).unwrap();
        assert_eq!(auto.account, "work");
        assert_eq!(
            auto.argv(["--resume".into()]),
            ["env", "CLAUDE_CONFIG_DIR=/p/work", "claude", "--resume"]
        );
        route.choice = SpawnAccount::Named("home@claude".into());
        assert_eq!(decide_with(&route, &[], never).unwrap().account, "home");
        route.choice = SpawnAccount::Named("nope".into());
        assert!(matches!(
            decide_with(&route, &[], never),
            Err(NotRouted::Bare(reason)) if reason.contains("no claude account named nope")
        ));
    }

    #[test]
    fn auto_takes_the_best_ranked_configured_account_and_refuses_when_all_are_held() {
        let mut route = route(SpawnAccount::Auto);
        route.profiles = vec![profile("work"), profile("home")];
        let now = time::OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let mut usage = crate::account_usage::Ledger::default();
        usage
            .accounts
            .entry("claude/work".into())
            .or_default()
            .hold("test", now + time::Duration::hours(1));
        assert_eq!(
            from_profiles_with(&route, &[], &usage, now)
                .unwrap()
                .account,
            "home"
        );
        // Seen capped by this launch: skipped like a held one.
        assert!(matches!(
            from_profiles_with(&route, &["home".into()], &usage, now),
            Err(NotRouted::Capped(_))
        ));
        usage
            .accounts
            .entry("claude/home".into())
            .or_default()
            .hold("test", now + time::Duration::hours(1));
        // Every account held capped: the agent would start into the limit.
        let Err(NotRouted::Capped(reason)) = from_profiles_with(&route, &[], &usage, now) else {
            panic!("all held must refuse");
        };
        assert!(reason.contains("work: capped until"), "{reason}");
        // A held pin refuses too, and says how to let tend choose.
        route.choice = SpawnAccount::Named("work".into());
        let Err(NotRouted::Capped(reason)) = from_profiles_with(&route, &[], &usage, now) else {
            panic!("a held pin must refuse");
        };
        assert!(reason.starts_with("work is at its usage limit"), "{reason}");
    }
}
