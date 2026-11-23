//! The CLI's door to the optional account module (`tend-aas`, the `aas`
//! cargo feature). Every caller goes through here, so a build or a config
//! without the module answers the same way: the launch starts bare and the
//! swap command says why.

use tend::swap_api::{AccountRoute, NotRouted, RoutedLaunch};

/// Whether this build carries the account module.
pub const COMPILED: bool = cfg!(feature = "aas");

/// Where a new agent starts, skipping `passed_over` (accounts this launch
/// already saw at their limit). `Err` is why it starts bare, or why it
/// does not start at all.
pub fn decide(route: &AccountRoute, passed_over: &[String]) -> Result<RoutedLaunch, NotRouted> {
    #[cfg(feature = "aas")]
    {
        tend_aas::account_route::decide(route, passed_over)
    }
    #[cfg(not(feature = "aas"))]
    {
        let _ = (route, passed_over);
        Err(NotRouted::Bare(tend::swap_api::module_unavailable(false)))
    }
}

/// Why `tend swap` and friends refuse, when they do: the module is not in
/// this build, or `config` does not turn it on.
pub fn unavailable(config: &tend::Config) -> Option<String> {
    if !COMPILED {
        return Some(tend::swap_api::module_unavailable(false));
    }
    (!config.accounts_module().enabled).then(|| tend::swap_api::module_unavailable(true))
}
