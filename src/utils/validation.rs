use std::{fmt, net::IpAddr, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    Hostname(String),
    Ip(IpAddr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    Empty,
    Whitespace,
    InvalidDomain,
    InvalidHost,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("input must not be empty"),
            Self::Whitespace => f.write_str("input must not contain whitespace"),
            Self::InvalidDomain => f.write_str("enter a valid public DNS domain name"),
            Self::InvalidHost => f.write_str("enter a valid hostname or IP address"),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Validates and normalizes a public DNS domain name (not an IP or local/special name).
pub fn validate_domain(input: &str) -> Result<String, ValidationError> {
    let domain = validate_dns_name(input)?;
    if domain.split('.').count() < 2 || is_local_or_special_domain(&domain) {
        return Err(ValidationError::InvalidDomain);
    }
    Ok(domain)
}

/// Validates an IP literal or syntactically valid DNS hostname, including single labels.
pub fn validate_host(input: &str) -> Result<Host, ValidationError> {
    if input.is_empty() {
        return Err(ValidationError::Empty);
    }
    if input.chars().any(char::is_whitespace) {
        return Err(ValidationError::Whitespace);
    }
    if let Ok(ip) = IpAddr::from_str(input) {
        return Ok(Host::Ip(ip));
    }
    if looks_like_malformed_ipv4(input) {
        return Err(ValidationError::InvalidHost);
    }
    validate_dns_name(input)
        .map(Host::Hostname)
        .map_err(|_| ValidationError::InvalidHost)
}

fn validate_dns_name(input: &str) -> Result<String, ValidationError> {
    if input.is_empty() {
        return Err(ValidationError::Empty);
    }
    if input.chars().any(char::is_whitespace) {
        return Err(ValidationError::Whitespace);
    }

    let domain = input.strip_suffix('.').unwrap_or(input);
    if domain.is_empty() || domain.len() > 253 || domain.parse::<IpAddr>().is_ok() {
        return Err(ValidationError::InvalidDomain);
    }
    if looks_like_malformed_ipv4(domain) {
        return Err(ValidationError::InvalidDomain);
    }

    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(ValidationError::InvalidDomain);
        }
        let bytes = label.as_bytes();
        if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
            return Err(ValidationError::InvalidDomain);
        }
        if !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            return Err(ValidationError::InvalidDomain);
        }
    }

    Ok(domain.to_ascii_lowercase())
}

fn is_local_or_special_domain(domain: &str) -> bool {
    [
        "localhost",
        "local",
        "test",
        "invalid",
        "example",
        "arpa",
        "alt",
        "onion",
        "internal",
        "example.com",
        "example.net",
        "example.org",
    ]
    .iter()
    .any(|suffix| domain == *suffix || domain.ends_with(&format!(".{suffix}")))
}

fn looks_like_malformed_ipv4(input: &str) -> bool {
    let parts: Vec<_> = input.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_domains_and_normalizes_case_and_root_dot() {
        assert_eq!(validate_domain("Acme.NET").unwrap(), "acme.net");
        assert_eq!(
            validate_domain("sub-domain.acme.net.").unwrap(),
            "sub-domain.acme.net"
        );
    }

    #[test]
    fn rejects_malformed_domains_and_shell_metacharacters() {
        for invalid in [
            "",
            " example.com",
            "example.com ",
            "a..b",
            ".example.com",
            "example..",
            "-bad.example",
            "bad-.example",
            "toolonglabeltoolonglabeltoolonglabeltoolonglabeltoolonglabeltoolonglabel.example",
            "a_b.example",
            "example.com;id",
            "$(whoami).example",
            "a|b.example",
            "a&b.example",
            "example.com/../x",
            "999.1.1.1",
            "127.0.0.1",
            "2001:db8::1",
            "éxample.com",
        ] {
            assert!(validate_domain(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn rejects_single_label_and_local_or_reserved_domains() {
        for invalid in [
            "localhost",
            "LOCALHOST.",
            "api.localhost",
            "host.local",
            "host.test",
            "host.invalid",
            "host.example",
            "hidden.onion",
            "service.internal",
            "foo.example.com",
            "example.com",
            "sub.example.net",
            "www.example.org",
            "resolver.arpa",
            "ipv4only.arpa",
            "service.arpa",
            "6tisch.arpa",
            "foo.alt",
            "alt",
            "example",
            "test",
        ] {
            assert!(validate_domain(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn accepts_hostname_and_ipv4_ipv6_hosts_including_single_labels() {
        assert_eq!(
            validate_host("Example.COM").unwrap(),
            Host::Hostname("example.com".to_owned())
        );
        assert_eq!(
            validate_host("node-1").unwrap(),
            Host::Hostname("node-1".to_owned())
        );
        assert_eq!(
            validate_host("localhost").unwrap(),
            Host::Hostname("localhost".to_owned())
        );
        assert_eq!(
            validate_host("192.0.2.10").unwrap(),
            Host::Ip("192.0.2.10".parse().unwrap())
        );
        assert_eq!(
            validate_host("2001:db8::1").unwrap(),
            Host::Ip("2001:db8::1".parse().unwrap())
        );
        assert_eq!(
            validate_host("::1").unwrap(),
            Host::Ip("::1".parse().unwrap())
        );
    }

    #[test]
    fn rejects_empty_whitespace_and_malformed_hosts() {
        for invalid in [
            "",
            "host name",
            "host\nname",
            "999.1.1.1",
            "a..b",
            "-bad.example",
            "host;id",
            "--",
            "[2001:db8::1]",
        ] {
            assert!(validate_host(invalid).is_err(), "accepted {invalid:?}");
        }
    }
}
