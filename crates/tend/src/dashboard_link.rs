//! Where a person opens this host's dashboard, so a client that cannot take
//! a decision itself can send them there. Tend for Mac uses it for review
//! items on other Fleet hosts: the person decides them on the dashboard,
//! under its sign-in, and the app never decides them over IPC.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};

/// The dashboard address tendd names on `hello`, shared between the IPC
/// server and the dashboard. An address the person configured is set at
/// start; the running dashboard's own address only once it is listening
/// (the port it actually got), and cleared when it stops.
#[derive(Clone, Debug, Default)]
pub struct DashboardLink(Arc<RwLock<Option<String>>>);

impl DashboardLink {
    #[must_use]
    pub fn new(origin: Option<String>) -> Self {
        Self(Arc::new(RwLock::new(origin)))
    }

    #[must_use]
    pub fn get(&self) -> Option<String> {
        self.0.read().map_or(None, |origin| origin.clone())
    }

    pub fn set(&self, origin: Option<String>) {
        if let Ok(mut current) = self.0.write() {
            *current = origin;
        }
    }
}

/// The dashboard's origin (`scheme://host[:port]`, no path or credentials):
/// `configured` (`[sinks.webhook] dashboard_url`) when it is an http(s) URL,
/// else the sign-in public origin, else the address the running dashboard
/// is bound to (loopback for an unspecified bind). `None` when there is no
/// dashboard to open.
#[must_use]
pub fn origin(
    configured: Option<&str>,
    public_origin: Option<&str>,
    bound: Option<SocketAddr>,
) -> Option<String> {
    let parsed = |url: &str| {
        url::Url::parse(url.trim())
            .ok()
            .filter(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some())
            .map(|url| url.origin().ascii_serialization())
    };
    configured
        .and_then(parsed)
        .or_else(|| public_origin.and_then(parsed))
        .or_else(|| {
            let mut bound = bound?;
            match bound.ip() {
                IpAddr::V4(ip) if ip.is_unspecified() => bound.set_ip(Ipv4Addr::LOCALHOST.into()),
                IpAddr::V6(ip) if ip.is_unspecified() => bound.set_ip(Ipv6Addr::LOCALHOST.into()),
                _ => {}
            }
            parsed(&format!("http://{bound}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configured_address_then_sign_in_then_the_bind() {
        let bind: SocketAddr = "127.0.0.1:7878".parse().unwrap();
        assert_eq!(
            origin(
                Some("https://tend.example.com/needs?x=1"),
                Some("https://login.example.com"),
                Some(bind)
            )
            .as_deref(),
            Some("https://tend.example.com")
        );
        assert_eq!(
            origin(None, Some("https://login.example.com"), Some(bind)).as_deref(),
            Some("https://login.example.com")
        );
        // Not a web address: the next one.
        assert_eq!(
            origin(Some(""), None, Some(bind)).as_deref(),
            Some("http://127.0.0.1:7878")
        );
        assert_eq!(origin(Some("javascript:alert(1)"), None, None), None);
        // Credentials never travel in the link.
        assert_eq!(
            origin(Some("https://user:secret@tend.example.com/"), None, None).as_deref(),
            Some("https://tend.example.com")
        );
        assert_eq!(origin(None, None, None), None);
    }

    #[test]
    fn the_link_is_shared_and_cleared() {
        let link = DashboardLink::new(None);
        let seen = link.clone();
        assert_eq!(seen.get(), None);
        link.set(origin(None, None, Some("127.0.0.1:41234".parse().unwrap())));
        assert_eq!(seen.get().as_deref(), Some("http://127.0.0.1:41234"));
        link.set(None);
        assert_eq!(seen.get(), None);
    }

    #[test]
    fn an_unspecified_bind_is_opened_on_loopback() {
        assert_eq!(
            origin(None, None, Some("0.0.0.0:7878".parse().unwrap())).as_deref(),
            Some("http://127.0.0.1:7878")
        );
        assert_eq!(
            origin(None, None, Some("[::]:7878".parse().unwrap())).as_deref(),
            Some("http://[::1]:7878")
        );
    }
}
