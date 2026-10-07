use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use ipnet::{Ipv4Net, Ipv6Net};
use reqwest::{Client, Url, redirect::Policy};
use tokio::{net::lookup_host, time::timeout};

/// Maximum time spent resolving a hostname before rejecting the URL.
pub const DNS_RESOLUTION_TIMEOUT: Duration = Duration::from_secs(5);

/// URL-validation failures intentionally contain no user-supplied host or address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeUrlError {
    InvalidUrl,
    UnsupportedScheme,
    CredentialsNotAllowed,
    ResolutionFailed,
    NoAddresses,
    NonPublicAddress,
}

impl fmt::Display for SafeUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidUrl => "enter a valid HTTP or HTTPS URL",
            Self::UnsupportedScheme => "only HTTP and HTTPS URLs are allowed",
            Self::CredentialsNotAllowed => "URLs containing credentials are not allowed",
            Self::ResolutionFailed => "the URL host could not be resolved safely",
            Self::NoAddresses => "the URL host did not resolve to any addresses",
            Self::NonPublicAddress => "the URL resolves to a non-public network address",
        })
    }
}

impl std::error::Error for SafeUrlError {}

/// Safe, host-free outcomes for a request made through a validated URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeRequestError {
    Timeout,
    Network,
    ClientBuild,
}

impl fmt::Display for SafeRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Timeout => "the request timed out",
            Self::Network => "the request failed due to a network error",
            Self::ClientBuild => "a safe HTTP client could not be created",
        })
    }
}

impl std::error::Error for SafeRequestError {}

/// A URL whose complete DNS answer set has been checked and pinned for requests.
///
/// Requests can only be made to the stored URL through [`SafeHttpUrl::get`].
/// The internal client pins validated resolutions, disables redirects and proxies,
/// and applies the supplied timeout.
#[derive(Debug, Clone)]
pub struct SafeHttpUrl {
    /// The parsed, normalized HTTP(S) URL supplied for the request.
    url: Url,
    /// Every checked resolution, preserving the URL's effective port.
    addresses: Vec<SocketAddr>,
}

impl SafeHttpUrl {
    /// Returns the immutable parsed URL that was validated.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Returns every immutable socket address validated during resolution.
    pub fn addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }

    /// Issues a GET only to this validated URL using its pinned DNS answers.
    pub async fn get(
        &self,
        request_timeout: Duration,
    ) -> Result<reqwest::Response, SafeRequestError> {
        let client = self.build_client(request_timeout)?;
        self.get_request(&client).send().await.map_err(|error| {
            if error.is_timeout() {
                SafeRequestError::Timeout
            } else {
                SafeRequestError::Network
            }
        })
    }

    /// Builds a client that is never exposed to callers and is scoped to this URL.
    fn build_client(&self, request_timeout: Duration) -> Result<Client, SafeRequestError> {
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(request_timeout);

        if let Some(hostname) = self.url.domain() {
            builder = builder.resolve_to_addrs(hostname, &self.addresses);
        }

        builder.build().map_err(|_| SafeRequestError::ClientBuild)
    }

    fn get_request(&self, client: &Client) -> reqwest::RequestBuilder {
        client.get(self.url.clone())
    }
}

/// Parses, resolves, and validates an HTTP(S) URL before any connection is made.
///
/// All DNS answers must be public; a mixed public/private answer set is rejected.
/// Use [`SafeHttpUrl::get`] so validated hostname resolutions cannot be looked up
/// again at connect time, and no alternate destination or redirect is requested.
pub async fn resolve_safe_http_url(input: &str) -> Result<SafeHttpUrl, SafeUrlError> {
    let url = parse_http_url(input)?;
    let port = url
        .port_or_known_default()
        .ok_or(SafeUrlError::InvalidUrl)?;

    let addresses = if let Some(hostname) = url.domain() {
        let resolved = timeout(DNS_RESOLUTION_TIMEOUT, lookup_host((hostname, port)))
            .await
            .map_err(|_| SafeUrlError::ResolutionFailed)?
            .map_err(|_| SafeUrlError::ResolutionFailed)?;
        let mut addresses: Vec<_> = resolved.collect();
        addresses.sort_unstable();
        addresses.dedup();
        addresses
    } else {
        let host = url.host_str().ok_or(SafeUrlError::InvalidUrl)?;
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        let ip = literal
            .parse::<IpAddr>()
            .map_err(|_| SafeUrlError::InvalidUrl)?;
        vec![SocketAddr::new(ip, port)]
    };

    if addresses.is_empty() {
        return Err(SafeUrlError::NoAddresses);
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(SafeUrlError::NonPublicAddress);
    }

    Ok(SafeHttpUrl { url, addresses })
}

fn parse_http_url(input: &str) -> Result<Url, SafeUrlError> {
    let url = Url::parse(input).map_err(|_| SafeUrlError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SafeUrlError::UnsupportedScheme);
    }
    // Also catch an empty userinfo section (`http://@host`), for which username()
    // alone is empty even though the URL still contains a credential delimiter.
    let has_userinfo_delimiter = input
        .split_once("://")
        .and_then(|(_, authority_and_path)| authority_and_path.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains('@'));
    if !url.username().is_empty() || url.password().is_some() || has_userinfo_delimiter {
        return Err(SafeUrlError::CredentialsNotAllowed);
    }
    if url.host().is_none() {
        return Err(SafeUrlError::InvalidUrl);
    }
    Ok(url)
}

/// Returns true only for addresses suitable as public Internet destinations.
///
/// This deliberately rejects special-purpose, documentation, transition, and
/// reserved ranges as well as private, loopback, link-local, and multicast space.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4() {
            Some(mapped) => is_public_ipv4(mapped),
            None => is_public_ipv6(ip),
        },
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    const NON_PUBLIC: &[&str] = &[
        "0.0.0.0/8",       // this network / unspecified
        "10.0.0.0/8",      // private
        "100.64.0.0/10",   // shared address space
        "127.0.0.0/8",     // loopback
        "169.254.0.0/16",  // link-local
        "172.16.0.0/12",   // private
        "192.0.0.0/24",    // IETF protocol assignments
        "192.0.2.0/24",    // documentation
        "192.88.99.0/24",  // deprecated 6to4 relay anycast
        "192.168.0.0/16",  // private
        "198.18.0.0/15",   // benchmarking
        "198.51.100.0/24", // documentation
        "203.0.113.0/24",  // documentation
        "224.0.0.0/4",     // multicast
        "240.0.0.0/4",     // reserved / broadcast
    ];

    !NON_PUBLIC.iter().any(|cidr| {
        cidr.parse::<Ipv4Net>()
            .is_ok_and(|network| network.contains(&ip))
    })
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    const GLOBAL_UNICAST: &str = "2000::/3";
    const NON_PUBLIC: &[&str] = &[
        "2001::/23",      // special protocol assignments, including Teredo/ORCHID
        "2001:db8::/32",  // documentation
        "2002::/16",      // deprecated 6to4 transition space
        "3ffe::/16",      // returned 6bone address space
        "3fff::/20",      // documentation
        "64:ff9b::/96",   // well-known NAT64 (may embed a non-public IPv4)
        "64:ff9b:1::/48", // local-use NAT64
        "100::/64",       // discard-only
        "5f00::/16",      // segment routing special-purpose
    ];

    GLOBAL_UNICAST
        .parse::<Ipv6Net>()
        .is_ok_and(|network| network.contains(&ip))
        && !NON_PUBLIC.iter().any(|cidr| {
            cidr.parse::<Ipv6Net>()
                .is_ok_and(|network| network.contains(&ip))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_http_and_https_urls_without_credentials() {
        assert_eq!(
            parse_http_url("https://example.com/a").unwrap().scheme(),
            "https"
        );
        assert_eq!(
            parse_http_url("http://example.com").unwrap().scheme(),
            "http"
        );
        for invalid in [
            "not a URL",
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert!(parse_http_url(invalid).is_err(), "accepted {invalid:?}");
        }
        for credentialed in [
            "http://user:password@example.com/",
            "https://user@example.com/",
            "http://@example.com/",
        ] {
            assert_eq!(
                parse_http_url(credentialed),
                Err(SafeUrlError::CredentialsNotAllowed),
                "accepted {credentialed:?}"
            );
        }
    }

    #[test]
    fn checks_public_and_non_public_ipv4_destinations() {
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        for address in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(
                !is_public_ip(address.parse().unwrap()),
                "accepted {address}"
            );
        }
    }

    #[test]
    fn checks_public_and_non_public_ipv6_destinations_and_mapped_ipv4() {
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
        for address in [
            "::",
            "::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "2001::1",
            "2002::1",
            "3ffe::1",
            "3fff::1",
            "64:ff9b::808:808",
            "100::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                !is_public_ip(address.parse().unwrap()),
                "accepted {address}"
            );
        }
        assert!(is_public_ip("::ffff:8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_non_public_ip_literals_before_connecting() {
        for input in [
            "http://127.0.0.1/",
            "https://[::1]/",
            "http://[::ffff:10.0.0.1]/",
        ] {
            assert_eq!(
                resolve_safe_http_url(input).await.unwrap_err(),
                SafeUrlError::NonPublicAddress
            );
        }
    }

    #[tokio::test]
    async fn public_ip_literal_returns_checked_url_and_address() {
        let safe = resolve_safe_http_url("https://8.8.8.8/dns-query")
            .await
            .unwrap();
        assert_eq!(safe.url().host_str(), Some("8.8.8.8"));
        assert_eq!(safe.addresses(), &["8.8.8.8:443".parse().unwrap()]);
        assert!(safe.build_client(Duration::from_secs(10)).is_ok());
    }

    #[test]
    fn request_construction_targets_only_the_validated_url_without_network_io() {
        let safe = SafeHttpUrl {
            url: Url::parse("https://8.8.8.8/checked/path").unwrap(),
            addresses: vec!["8.8.8.8:443".parse().unwrap()],
        };
        let client = safe.build_client(Duration::from_secs(10)).unwrap();
        let request = safe.get_request(&client).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(request.url(), safe.url());
    }

    #[test]
    fn errors_never_echo_private_addresses_or_input() {
        let error = SafeUrlError::NonPublicAddress.to_string();
        assert!(!error.contains("127.0.0.1"));
        assert!(!error.contains("10.0.0.1"));
    }
}
