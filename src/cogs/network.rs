use std::time::{Duration, Instant};
#[cfg(target_os = "linux")]
use std::{io::ErrorKind, process::Stdio};

use hickory_resolver::TokioAsyncResolver;
use poise::serenity_prelude::UserId;
#[cfg(target_os = "linux")]
use tokio::process::Command;
use tokio::{net::TcpStream, time::timeout};

use crate::{CooldownCheck, Data, Error};
use lime_discord_bot::utils::{
    Host, SafeRequestError, format_code_block, is_public_ip, resolve_safe_http_url, validate_host,
};

const CURL_TIMEOUT: Duration = Duration::from_secs(10);
const CURL_BODY_LIMIT: usize = 32 * 1024;
const CURL_FILE_BODY_LIMIT: usize = 5 * 1024 * 1024;
const CURL_MAX_REDIRECTS: usize = 5;
const CURL_COOLDOWN: Duration = Duration::from_secs(10);
const PING_COOLDOWN: Duration = Duration::from_secs(5);
const PING_PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const PING_DNS_TIMEOUT: Duration = Duration::from_secs(5);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const PROBE_NOTICE: &str = "Note: this command probes one specified host and port per invocation; use it only for destinations you are authorized to test.";

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![curl(), ping()]
}

#[poise::command(slash_command)]
async fn curl(
    ctx: poise::Context<'_, Data, Error>,
    url: String,
    #[description = "Include response headers with the body, like curl -i."]
    include_headers: Option<bool>,
    #[description = "Use HTTP HEAD and show headers only, like curl -I."] head_only: Option<bool>,
    #[description = "Omit status and timing details; errors are still reported, like curl -s."]
    silent: Option<bool>,
    #[description = "Follow up to five redirects after validating every destination, like curl -L."]
    follow_redirects: Option<bool>,
    #[description = "Attach the response using this filename for download, like curl -o."]
    output_file: Option<String>,
) -> Result<(), Error> {
    if let Some(reply) = cooldown_reply(ctx.data(), ctx.author().id, "curl", CURL_COOLDOWN) {
        ctx.say(reply).await?;
        return Ok(());
    }

    // DNS and HTTP requests can each take longer than Discord's 3-second
    // initial-response deadline, so acknowledge before beginning network I/O.
    ctx.defer().await?;

    let started = Instant::now();
    let head_only = head_only.unwrap_or(false);
    let include_headers = include_headers.unwrap_or(false) || head_only;
    let silent = silent.unwrap_or(false);
    let follow_redirects = follow_redirects.unwrap_or(false);
    let output_file = output_file.map(|filename| safe_attachment_filename(&filename));
    let body_limit = if output_file.is_some() {
        CURL_FILE_BODY_LIMIT
    } else {
        CURL_BODY_LIMIT
    };
    let safe_url = match resolve_safe_http_url(&url, &ctx.data().resolver).await {
        Ok(url) => url,
        Err(error) => {
            eprintln!("[lime][curl] URL validation/resolution failed: {error:?}");
            ctx.say(format_code_block("Curl error", &error.to_string()))
                .await?;
            return Ok(());
        }
    };
    let mut safe_url = safe_url;
    let mut redirects_followed = 0;
    let mut response = loop {
        let request = if head_only {
            safe_url.head(CURL_TIMEOUT).await
        } else {
            safe_url.get(CURL_TIMEOUT).await
        };
        let response = match request {
            Ok(response) => response,
            Err(error) => {
                ctx.say(format_code_block(
                    "Curl error",
                    request_error_message(error),
                ))
                .await?;
                return Ok(());
            }
        };
        if !follow_redirects || !is_followable_redirect(response.status()) {
            break response;
        }
        let location = match response.headers().get(reqwest::header::LOCATION) {
            Some(location) => match location.to_str() {
                Ok(location) => location.to_owned(),
                Err(_) => {
                    let host = safe_url.url().host_str().unwrap_or("<unknown>");
                    eprintln!("[lime][curl] invalid redirect Location header host={host}");
                    ctx.say(format_code_block(
                        "Curl error",
                        "the redirect contained an invalid Location header",
                    ))
                    .await?;
                    return Ok(());
                }
            },
            None => break response,
        };
        if redirects_followed >= CURL_MAX_REDIRECTS {
            let host = safe_url.url().host_str().unwrap_or("<unknown>");
            eprintln!(
                "[lime][curl] redirect limit exceeded host={host} limit={CURL_MAX_REDIRECTS}"
            );
            ctx.say(format_code_block(
                "Curl error",
                "the redirect limit (5) was exceeded",
            ))
            .await?;
            return Ok(());
        }
        let next_url = match safe_url.url().join(&location) {
            Ok(url) => url,
            Err(_) => {
                let host = safe_url.url().host_str().unwrap_or("<unknown>");
                eprintln!("[lime][curl] malformed redirect target host={host}");
                ctx.say(format_code_block(
                    "Curl error",
                    "the redirect target was not a valid HTTP or HTTPS URL",
                ))
                .await?;
                return Ok(());
            }
        };
        drop(response);
        safe_url = match resolve_safe_http_url(next_url.as_str(), &ctx.data().resolver).await {
            Ok(url) => url,
            Err(error) => {
                let host = next_url.host_str().unwrap_or("<unknown>");
                eprintln!("[lime][curl] redirect target rejected host={host}: {error}");
                ctx.say(format_code_block(
                    "Curl error",
                    &format!("redirect target rejected: {error}"),
                ))
                .await?;
                return Ok(());
            }
        };
        redirects_followed += 1;
    };
    let status = response.status();
    let headers = if include_headers {
        format_response_headers(response.headers())
    } else {
        String::new()
    };
    if !head_only
        && response
            .content_length()
            .is_some_and(|length| length > body_limit as u64)
    {
        eprintln!(
            "[lime][curl] declared response body exceeds limit={} bytes; keeping a bounded prefix",
            body_limit
        );
    }

    let mut body = Vec::with_capacity(body_limit.min(8 * 1024));
    let mut body_truncated = false;
    if !head_only {
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    let is_timeout = error.is_timeout();
                    let host = safe_url.url().host_str().unwrap_or("<unknown>");
                    eprintln!(
                        "[lime][curl] response body read failed host={host}: {}",
                        error.without_url()
                    );
                    let message = if is_timeout {
                        "the response body read timed out"
                    } else {
                        "the response body could not be read due to a network error"
                    };
                    ctx.say(format_code_block("Curl error", message)).await?;
                    return Ok(());
                }
            };
            if append_up_to_limit(&mut body, &chunk, body_limit) {
                body_truncated = true;
                eprintln!(
                    "[lime][curl] streamed response truncated at limit={} bytes",
                    body_limit
                );
                break;
            }
        }
    }

    let elapsed_ms = started.elapsed().as_millis();
    let mut content = String::new();
    if !silent {
        content.push_str(&format!(
            "Status: {}\nElapsed: {elapsed_ms} ms",
            status.as_u16()
        ));
        if redirects_followed > 0 {
            content.push_str(&format!("\nRedirects followed: {redirects_followed}"));
        }
    }
    if include_headers {
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str("Response headers:\n");
        content.push_str(&headers);
    }
    if body_truncated {
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str(&format!("[Response truncated at {body_limit} bytes]"));
    }
    let title = if head_only {
        "HTTP HEAD result"
    } else {
        "HTTP GET result"
    };
    if let Some(filename) = output_file.as_deref() {
        let mut file_body = Vec::with_capacity(body.len() + headers.len() + 128);
        if include_headers {
            file_body.extend_from_slice(format!("Status: {}\r\n", status.as_u16()).as_bytes());
            file_body.extend_from_slice(headers.as_bytes());
            file_body.extend_from_slice(b"\r\n\r\n");
        }
        if !head_only {
            file_body.extend_from_slice(&body);
        }
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str(&format!("Response attached as `{filename}`."));
        let attachment =
            poise::serenity_prelude::CreateAttachment::bytes(file_body, filename.to_owned());
        ctx.send(
            poise::CreateReply::default()
                .content(format_code_block(title, &content))
                .attachment(attachment),
        )
        .await?;
    } else {
        if !head_only {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&String::from_utf8_lossy(&body));
        }
        if content.is_empty() {
            content.push_str("(empty response body)");
        }
        ctx.say(format_code_block(title, &content)).await?;
    }
    Ok(())
}

fn is_followable_redirect(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn safe_attachment_filename(input: &str) -> String {
    let leaf = input.rsplit(['/', '\\']).next().unwrap_or("");
    let sanitized: String = leaf
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        "curl-response.bin".to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn format_response_headers(headers: &reqwest::header::HeaderMap) -> String {
    let lines: Vec<String> = headers
        .iter()
        .map(|(name, value)| {
            let value = if matches!(
                name.as_str().to_ascii_lowercase().as_str(),
                "set-cookie" | "authorization" | "proxy-authorization" | "www-authenticate"
            ) {
                "[redacted]"
            } else {
                value.to_str().unwrap_or("[non-text value]")
            };
            format!("{name}: {value}")
        })
        .collect();
    if lines.is_empty() {
        "(no response headers)".to_owned()
    } else {
        lines.join("\n")
    }
}

#[poise::command(slash_command)]
async fn ping(
    ctx: poise::Context<'_, Data, Error>,
    host: String,
    port: Option<u16>,
) -> Result<(), Error> {
    if let Some(reply) = cooldown_reply(ctx.data(), ctx.author().id, "ping", PING_COOLDOWN) {
        ctx.say(reply).await?;
        return Ok(());
    }

    let host = match validate_host(&host) {
        Ok(host) => host,
        Err(error) => {
            ctx.say(format!("Invalid host: {error}.\n{PROBE_NOTICE}"))
                .await?;
            return Ok(());
        }
    };
    let port = port.unwrap_or(443);
    if port == 0 {
        ctx.say(format!("Port must be between 1 and 65535.\n{PROBE_NOTICE}"))
            .await?;
        return Ok(());
    }

    // ICMP may wait up to 5 seconds; the TCP fallback may wait up to 4.
    ctx.defer().await?;

    let host = match resolve_public_host(&host, port, &ctx.data().resolver).await {
        Ok(host) => host,
        Err(error) => {
            eprintln!("[lime][ping] host resolution/safety check failed: {error:?}");
            let message = match error {
                ProbeHostError::ResolutionFailed => "The host could not be resolved safely.",
                ProbeHostError::NoAddresses => "The host did not resolve to an address.",
                ProbeHostError::NonPublicAddress => {
                    "Only public Internet destinations are allowed."
                }
            };
            ctx.say(format!("{message}\n{PROBE_NOTICE}")).await?;
            return Ok(());
        }
    };

    #[cfg(target_os = "linux")]
    let result = match run_icmp(&host).await {
        IcmpCheck::Succeeded => {
            "ICMP ping succeeded (the system ping process received a response).".to_owned()
        }
        IcmpCheck::Failed => "ICMP ping failed (the ping process reported failure).".to_owned(),
        IcmpCheck::TimedOut => "ICMP ping timed out after 5 seconds.".to_owned(),
        IcmpCheck::Unavailable => tcp_check(&host, port).await,
    };
    #[cfg(not(target_os = "linux"))]
    let result = tcp_check(&host, port).await;

    ctx.say(format!("{result}\n\n{PROBE_NOTICE}")).await?;
    Ok(())
}

fn cooldown_reply(
    data: &Data,
    user_id: UserId,
    command: &'static str,
    duration: Duration,
) -> Option<String> {
    match data.check_cooldown(user_id.get(), command, duration) {
        CooldownCheck::Allowed => None,
        CooldownCheck::Wait(remaining) => Some(format!(
            "Please wait {} second(s) before using this network command again.",
            remaining.as_secs().max(1)
        )),
        CooldownCheck::AtCapacity => {
            Some("Network command cooldowns are at capacity; please try again shortly.".to_owned())
        }
    }
}

fn request_error_message(error: SafeRequestError) -> &'static str {
    match error {
        SafeRequestError::Timeout => "the request timed out",
        SafeRequestError::Network => "the request failed due to a network error",
        SafeRequestError::ClientBuild => "a safe HTTP client could not be created",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeHostError {
    ResolutionFailed,
    NoAddresses,
    NonPublicAddress,
}

/// Resolves once, rejects mixed/private answers, and returns a pinned public IP.
async fn resolve_public_host(
    host: &Host,
    port: u16,
    resolver: &TokioAsyncResolver,
) -> Result<Host, ProbeHostError> {
    match host {
        Host::Ip(address) => {
            if is_public_ip(*address) {
                Ok(Host::Ip(*address))
            } else {
                Err(ProbeHostError::NonPublicAddress)
            }
        }
        Host::Hostname(hostname) => {
            let resolved =
                match timeout(PING_DNS_TIMEOUT, resolver.lookup_ip(hostname.as_str())).await {
                    Err(_) => {
                        eprintln!(
                            "[lime][ping] DNS lookup timed out host={hostname} limit={}s",
                            PING_DNS_TIMEOUT.as_secs()
                        );
                        return Err(ProbeHostError::ResolutionFailed);
                    }
                    Ok(Err(error)) => {
                        eprintln!("[lime][ping] DNS lookup failed host={hostname}: {error}");
                        return Err(ProbeHostError::ResolutionFailed);
                    }
                    Ok(Ok(resolved)) => resolved,
                };
            let addresses = resolved
                .iter()
                .map(|ip| std::net::SocketAddr::new(ip, port))
                .collect::<Vec<_>>();
            pin_public_addresses(addresses).inspect_err(|error| {
                eprintln!("[lime][ping] rejected DNS answers host={hostname}: {error:?}");
            })
        }
    }
}

fn pin_public_addresses(addresses: Vec<std::net::SocketAddr>) -> Result<Host, ProbeHostError> {
    if addresses.is_empty() {
        return Err(ProbeHostError::NoAddresses);
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(ProbeHostError::NonPublicAddress);
    }

    Ok(Host::Ip(addresses[0].ip()))
}

fn append_up_to_limit(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> bool {
    let available = limit.saturating_sub(body.len());
    let appended = chunk.len().min(available);
    body.extend_from_slice(&chunk[..appended]);
    appended < chunk.len()
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IcmpCheck {
    Succeeded,
    Failed,
    TimedOut,
    Unavailable,
}

#[cfg(target_os = "linux")]
async fn run_icmp(host: &Host) -> IcmpCheck {
    let host = host_argument(host);
    let mut command = Command::new("ping");
    command
        .args(["-n", "-c", "3", "-W", "1", "--", &host])
        .stdin(Stdio::null())
        // Do not surface or buffer process diagnostics: command output is unnecessary
        // to determine success and could contain uncontrolled text.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied
            ) =>
        {
            eprintln!(
                "[lime][ping] ICMP ping executable unavailable ({error}); using TCP fallback"
            );
            return IcmpCheck::Unavailable;
        }
        Err(error) => {
            eprintln!("[lime][ping] could not start ICMP ping process: {error}");
            return IcmpCheck::Failed;
        }
    };

    match timeout(PING_PROCESS_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => IcmpCheck::Succeeded,
        Ok(Ok(status)) => {
            eprintln!("[lime][ping] ICMP process exited unsuccessfully: {status}");
            IcmpCheck::Failed
        }
        Ok(Err(error)) => {
            eprintln!("[lime][ping] waiting for ICMP process failed: {error}");
            IcmpCheck::Failed
        }
        Err(_) => {
            eprintln!(
                "[lime][ping] ICMP process timed out after {}s",
                PING_PROCESS_TIMEOUT.as_secs()
            );
            IcmpCheck::TimedOut
        }
    }
}

fn host_argument(host: &Host) -> String {
    match host {
        Host::Hostname(hostname) => hostname.clone(),
        Host::Ip(address) => address.to_string(),
    }
}

async fn tcp_check(host: &Host, port: u16) -> String {
    let result = timeout(TCP_CONNECT_TIMEOUT, async {
        match host {
            Host::Hostname(hostname) => TcpStream::connect((hostname.as_str(), port)).await,
            Host::Ip(address) => {
                TcpStream::connect(std::net::SocketAddr::new(*address, port)).await
            }
        }
    })
    .await;
    match result {
        Ok(Ok(_stream)) => "TCP connectivity check succeeded.".to_owned(),
        Ok(Err(error)) => {
            eprintln!("[lime][ping] TCP connection failed host={host:?} port={port}: {error}");
            "TCP connectivity check failed: the connection could not be established.".to_owned()
        }
        Err(_) => {
            eprintln!("[lime][ping] TCP connection timed out host={host:?} port={port}");
            "TCP connectivity check timed out after 4 seconds.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lime_discord_bot::utils::{MAX_DISCORD_MESSAGE_CHARS, SafeUrlError, escape_discord_text};

    #[test]
    fn bounded_body_append_accepts_exact_limit_without_truncating() {
        let mut body = Vec::new();
        assert!(!append_up_to_limit(&mut body, b"1234", 4));
        assert_eq!(body, b"1234");
    }

    #[test]
    fn file_response_limit_is_five_mib_while_inline_limit_stays_32_kib() {
        assert_eq!(CURL_FILE_BODY_LIMIT, 5 * 1024 * 1024);
        assert_eq!(CURL_BODY_LIMIT, 32 * 1024);
    }

    #[test]
    fn bounded_body_append_keeps_prefix_and_reports_truncation() {
        let mut body = b"123".to_vec();
        assert!(append_up_to_limit(&mut body, b"45", 4));
        assert_eq!(body, b"1234");
        assert!(append_up_to_limit(&mut body, b"6", 4));
        assert_eq!(body, b"1234");
    }

    #[tokio::test]
    async fn invalid_and_unsupported_urls_fail_before_any_network_access() {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
        assert_eq!(
            resolve_safe_http_url("not a URL", &resolver)
                .await
                .unwrap_err(),
            SafeUrlError::InvalidUrl
        );
        assert_eq!(
            resolve_safe_http_url("ftp://example.net/file", &resolver)
                .await
                .unwrap_err(),
            SafeUrlError::UnsupportedScheme
        );
    }

    #[test]
    fn curl_body_formatting_neutralizes_fences_and_mentions_within_message_budget() {
        let formatted = format_code_block(
            "HTTP GET result",
            &format!(
                "Status: 200\nElapsed: 2 ms\n{} @everyone ```",
                "🙂".repeat(2_000)
            ),
        );
        assert!(formatted.encode_utf16().count() <= MAX_DISCORD_MESSAGE_CHARS);
        assert!(!formatted.contains("@everyone"));
        assert!(!formatted.contains("```\nStatus"));
        assert!(formatted.ends_with("\n```"));
        assert_eq!(escape_discord_text("```").as_str(), "ˋˋˋ");
    }

    #[test]
    fn network_error_messages_never_include_request_details() {
        for error in [
            SafeRequestError::Timeout,
            SafeRequestError::Network,
            SafeRequestError::ClientBuild,
        ] {
            let message = request_error_message(error);
            assert!(!message.contains("https://"));
            assert!(!message.contains("127.0.0.1"));
        }
    }

    #[test]
    fn response_header_formatting_redacts_sensitive_values() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("content-type", "text/html".parse().unwrap());
        headers.insert("set-cookie", "session=private-value".parse().unwrap());
        let formatted = format_response_headers(&headers);
        assert!(formatted.contains("content-type: text/html"));
        assert!(formatted.contains("set-cookie: [redacted]"));
        assert!(!formatted.contains("private-value"));
    }

    #[test]
    fn follows_only_standard_redirect_status_codes() {
        for code in [301, 302, 303, 307, 308] {
            assert!(is_followable_redirect(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        for code in [200, 300, 304, 305] {
            assert!(!is_followable_redirect(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
    }

    #[test]
    fn attachment_filename_is_a_safe_leaf_name() {
        assert_eq!(
            safe_attachment_filename("../../my response.html"),
            "my_response.html"
        );
        assert_eq!(safe_attachment_filename(".."), "curl-response.bin");
        assert_eq!(safe_attachment_filename("\u{1b}"), "_");
    }

    #[tokio::test]
    async fn ping_rejects_private_loopback_and_mapped_private_ip_literals() {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
        for address in ["10.0.0.1", "127.0.0.1", "::1", "::ffff:192.168.1.1"] {
            let host = Host::Ip(address.parse().unwrap());
            assert_eq!(
                resolve_public_host(&host, 443, &resolver).await,
                Err(ProbeHostError::NonPublicAddress),
                "accepted {address}"
            );
        }
    }

    #[tokio::test]
    async fn ping_accepts_a_public_ip_literal_and_returns_it_pinned() {
        let resolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
        let address = "2606:4700:4700::1111".parse().unwrap();
        assert_eq!(
            resolve_public_host(&Host::Ip(address), 443, &resolver).await,
            Ok(Host::Ip(address))
        );
    }

    #[test]
    fn ping_rejects_a_mixed_public_and_private_dns_answer_set() {
        let addresses = vec![
            "8.8.8.8:443".parse().unwrap(),
            "10.0.0.1:443".parse().unwrap(),
        ];
        assert_eq!(
            pin_public_addresses(addresses),
            Err(ProbeHostError::NonPublicAddress)
        );
    }

    #[test]
    fn ping_pins_the_first_ip_after_all_dns_answers_pass_validation() {
        let addresses = vec![
            "8.8.8.8:443".parse().unwrap(),
            "1.1.1.1:443".parse().unwrap(),
        ];
        assert_eq!(
            pin_public_addresses(addresses),
            Ok(Host::Ip("8.8.8.8".parse().unwrap()))
        );
    }

    #[test]
    fn hostname_argument_is_validated_representation() {
        assert_eq!(
            host_argument(&Host::Hostname("example.net".into())),
            "example.net"
        );
        assert_eq!(
            host_argument(&Host::Ip("2001:db8::1".parse().unwrap()),),
            "2001:db8::1"
        );
    }
}
