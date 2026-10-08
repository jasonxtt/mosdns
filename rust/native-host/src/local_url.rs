//! How a plugin calls a URL that may be this host's own API.
//!
//! `domain_output` posts its rule list and `requery` calls save/flush URLs;
//! both go through [`LocalApi::call`], which serves a URL that names this
//! host's API through the same plugin routes the listener serves (no
//! connection), and sends any other URL over HTTP.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use mosdns_upstream_core::TransportCancellation;

use crate::runtime_snapshot::RuntimeControlWeak;

/// Where a URL action goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    /// This host's API: the path and query.
    Local(String),
    /// Any other URL.
    Http(String),
}

/// `domain_output` rule: the URL must name this host's API address and port.
pub(crate) fn exact(url: &str, api: Option<SocketAddr>) -> Target {
    let Some(api) = api else {
        return Target::Http(url.to_owned());
    };
    let Ok(parsed) = url::Url::parse(url) else {
        return Target::Http(url.to_owned());
    };
    let own = match parsed.host() {
        Some(url::Host::Domain(name)) => {
            name.eq_ignore_ascii_case("localhost")
                && (api.ip().is_loopback() || api.ip().is_unspecified())
        }
        Some(url::Host::Ipv4(ip)) => is_own(IpAddr::V4(ip), api.ip()),
        Some(url::Host::Ipv6(ip)) => is_own(IpAddr::V6(ip), api.ip()),
        None => false,
    };
    if parsed.scheme() == "http" && own && parsed.port_or_known_default() == Some(api.port()) {
        Target::Local(path_and_query(&parsed))
    } else {
        Target::Http(url.to_owned())
    }
}

/// `requery` rule (the reference `normalizeLocalActionURL`): a URL whose host
/// is `127.0.0.1`, `localhost`, `0.0.0.0`, `::` or `::1` names this host's API
/// whatever port it spells (the API port can be changed after the URL list was
/// written).
pub(crate) fn normalized(url: &str, api: Option<SocketAddr>) -> Target {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return Target::Http(url.to_owned());
    };
    let own = match parsed.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST || ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST || ip.is_unspecified(),
        None => false,
    };
    if api.is_some() && parsed.scheme() == "http" && own {
        Target::Local(path_and_query(&parsed))
    } else {
        Target::Http(url.to_owned())
    }
}

fn path_and_query(url: &url::Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// Whether `host` reaches this host's API socket: its own address, or any
/// loopback address when the API listens on the wildcard.
fn is_own(host: IpAddr, api: IpAddr) -> bool {
    host == api || (api.is_unspecified() && host.is_loopback())
}

/// What a URL action needs to run.
#[derive(Clone)]
pub(crate) struct LocalApi {
    pub(crate) control: RuntimeControlWeak,
    pub(crate) api: Option<SocketAddr>,
    pub(crate) roots: Option<Arc<rustls::RootCertStore>>,
}

impl LocalApi {
    /// Performs `method` on `target` and returns `(status, body)`.
    ///
    /// A local target is served in-process; a route the in-process entrance
    /// does not own is sent to this host's listener over loopback. `json` is
    /// the request body of a POST.
    ///
    /// # Errors
    ///
    /// A message for the log: the host is gone, no API listener, or the
    /// transport failed, timed out or was cancelled.
    pub(crate) async fn call(
        &self,
        method: &'static str,
        target: &Target,
        json: Option<&[u8]>,
        timeout: Duration,
        cancel: &TransportCancellation,
    ) -> Result<(u16, Vec<u8>), String> {
        let url = match target {
            Target::Local(path) => {
                let control = self
                    .control
                    .upgrade()
                    .ok_or_else(|| "the host is stopping".to_owned())?;
                if let Some(reply) = crate::api::local_plugin_request(
                    &control,
                    method,
                    path,
                    json.unwrap_or_default(),
                )
                .await
                {
                    return Ok(reply);
                }
                let api = self
                    .api
                    .ok_or_else(|| "the host has no api listener".to_owned())?;
                let ip = if api.ip().is_unspecified() {
                    IpAddr::from([127, 0, 0, 1])
                } else {
                    api.ip()
                };
                format!("http://{}{path}", SocketAddr::new(ip, api.port()))
            }
            Target::Http(url) => url.clone(),
        };
        crate::rule_download::call(
            crate::rule_download::Call {
                method,
                url: &url,
                body: json,
                socks5: "",
                timeout,
                max_bytes: 1024 * 1024,
                roots: self.roots.clone(),
                follow: method == "GET",
                error_body: true,
            },
            cancel,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(text: &str) -> Option<SocketAddr> {
        Some(text.parse().unwrap())
    }

    #[test]
    fn the_exact_rule_needs_this_apis_address_and_port() {
        let local = |path: &str| Target::Local(path.to_owned());
        let http = |url: &str| Target::Http(url.to_owned());
        let url = "http://127.0.0.1:9099/plugins/x/post?a=b";
        assert_eq!(
            exact(url, api("127.0.0.1:9099")),
            local("/plugins/x/post?a=b")
        );
        assert_eq!(
            exact(url, api("0.0.0.0:9099")),
            local("/plugins/x/post?a=b")
        );
        assert_eq!(exact(url, api("[::]:9099")), local("/plugins/x/post?a=b"));
        assert_eq!(exact(url, api("127.0.0.1:9100")), http(url));
        assert_eq!(exact(url, api("192.0.2.1:9099")), http(url));
        assert_eq!(exact(url, None), http(url));
        assert_eq!(
            exact("http://localhost:9099/p", api("127.0.0.1:9099")),
            local("/p")
        );
        assert_eq!(
            exact("http://127.0.0.2:9099/p", api("127.0.0.1:9099")),
            http("http://127.0.0.2:9099/p")
        );
        assert_eq!(
            exact("https://127.0.0.1:9099/p", api("127.0.0.1:9099")),
            http("https://127.0.0.1:9099/p")
        );
    }

    #[test]
    fn the_normalized_rule_ignores_the_spelled_port() {
        let local = |path: &str| Target::Local(path.to_owned());
        for host in [
            "127.0.0.1",
            "localhost",
            "0.0.0.0",
            "[::]",
            "[::1]",
            "LOCALHOST",
        ] {
            assert_eq!(
                normalized(
                    &format!("http://{host}:9099/plugins/c/save"),
                    api("127.0.0.1:8080")
                ),
                local("/plugins/c/save"),
                "{host}"
            );
        }
        // Only these exact hosts count, not the rest of 127.0.0.0/8.
        let loopback_neighbour = "http://127.0.0.2:9099/plugins/c/save";
        assert_eq!(
            normalized(loopback_neighbour, api("127.0.0.1:8080")),
            Target::Http(loopback_neighbour.to_owned())
        );
        let foreign = "http://192.0.2.1:9099/plugins/c/save";
        assert_eq!(
            normalized(foreign, api("127.0.0.1:8080")),
            Target::Http(foreign.to_owned())
        );
        assert_eq!(
            normalized("not a url", api("127.0.0.1:8080")),
            Target::Http("not a url".to_owned())
        );
    }
}
