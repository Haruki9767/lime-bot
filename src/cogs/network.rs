use std::time::{Duration, Instant};
#[cfg(target_os = "linux")]
use std::{io::ErrorKind, process::Stdio};

use poise::serenity_prelude::UserId;
#[cfg(target_os = "linux")]
use tokio::process::Command;
use tokio::{
    net::{TcpStream, lookup_host},
    time::timeout,
};

use crate::{CooldownCheck, Data, Error};
use lime_discord_bot::utils::{
    Host, SafeRequestError, format_code_block, is_public_ip, resolve_safe_http_url, validate_host,
};

const CURL_TIMEOUT: Duration = Duration::from_secs(10);
const CURL_BODY_LIMIT: usize = 32 * 1024;
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
async fn curl(ctx: poise::Context<'_, Data, Error>, url: String) -> Result<(), Error> {
    if let Some(reply) = cooldown_reply(ctx.data(), ctx.author().id, "curl", CURL_COOLDOWN) {
        ctx.say(reply).await?;
        return Ok(());
    }

    // DNS and HTTP requests can each take longer than Discord's 3-second
    // initial-response deadline, so acknowledge before beginning network I/O.
    ctx.defer().await?;

    let started = Instant::now();
    let safe_url = match resolve_safe_http_url(&url).await {
        Ok(url) => url,
        Err(error) => {
            ctx.say(format_code_block("Curl error", &error.to_string()))
                .await?;
            return Ok(());
        }
    };

    let mut response = match safe_url.get(CURL_TIMEOUT).await {
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
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > CURL_BODY_LIMIT as u64)
    {
        ctx.say(format_code_block(
            "Curl error",
            &format!("response body exceeds the {CURL_BODY_LIMIT}-byte limit"),
        ))
        .await?;
        return Ok(());
    }

    let mut body = Vec::with_capacity(CURL_BODY_LIMIT.min(8 * 1024));
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                let message = if error.is_timeout() {
                    "the response body read timed out"
                } else {
                    "the response body could not be read due to a network error"
                };
                ctx.say(format_code_block("Curl error", message)).await?;
                return Ok(());
            }
        };
        if append_bounded(&mut body, &chunk, CURL_BODY_LIMIT).is_err() {
            ctx.say(format_code_block(
                "Curl error",
                &format!("response body exceeds the {CURL_BODY_LIMIT}-byte limit"),
            ))
            .await?;
            return Ok(());
        }
    }

    let elapsed_ms = started.elapsed().as_millis();
    let body = String::from_utf8_lossy(&body);
    let content = format!(
        "Status: {}\nElapsed: {elapsed_ms} ms\n\n{body}",
        status.as_u16()
    );
    ctx.say(format_code_block("HTTP GET result", &content))
        .await?;
    Ok(())
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

    let host = match resolve_public_host(&host, port).await {
        Ok(host) => host,
        Err(error) => {
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
async fn resolve_public_host(host: &Host, port: u16) -> Result<Host, ProbeHostError> {
    match host {
        Host::Ip(address) => {
            if is_public_ip(*address) {
                Ok(Host::Ip(*address))
            } else {
                Err(ProbeHostError::NonPublicAddress)
            }
        }
        Host::Hostname(hostname) => {
            let addresses = timeout(PING_DNS_TIMEOUT, lookup_host((hostname.as_str(), port)))
                .await
                .map_err(|_| ProbeHostError::ResolutionFailed)?
                .map_err(|_| ProbeHostError::ResolutionFailed)?
                .collect::<Vec<_>>();
            pin_public_addresses(addresses)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BodyTooLarge;

fn append_bounded(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), BodyTooLarge> {
    if body.len() > limit || chunk.len() > limit - body.len() {
        return Err(BodyTooLarge);
    }
    body.extend_from_slice(chunk);
    Ok(())
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
            return IcmpCheck::Unavailable;
        }
        Err(_) => return IcmpCheck::Failed,
    };

    match timeout(PING_PROCESS_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => IcmpCheck::Succeeded,
        Ok(Ok(_)) | Ok(Err(_)) => IcmpCheck::Failed,
        Err(_) => IcmpCheck::TimedOut,
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
        Ok(Err(_)) => {
            "TCP connectivity check failed: the connection could not be established.".to_owned()
        }
        Err(_) => "TCP connectivity check timed out after 4 seconds.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lime_discord_bot::utils::{MAX_DISCORD_MESSAGE_CHARS, SafeUrlError, escape_discord_text};

    #[test]
    fn bounded_body_append_accepts_exact_limit() {
        let mut body = Vec::new();
        assert!(append_bounded(&mut body, b"1234", 4).is_ok());
        assert_eq!(body, b"1234");
    }

    #[test]
    fn bounded_body_append_rejects_limit_plus_one_without_appending() {
        let mut body = b"123".to_vec();
        assert_eq!(append_bounded(&mut body, b"45", 4), Err(BodyTooLarge));
        assert_eq!(body, b"123");
    }

    #[tokio::test]
    async fn invalid_and_unsupported_urls_fail_before_any_network_access() {
        assert_eq!(
            resolve_safe_http_url("not a URL").await.unwrap_err(),
            SafeUrlError::InvalidUrl
        );
        assert_eq!(
            resolve_safe_http_url("ftp://example.net/file")
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

    #[tokio::test]
    async fn ping_rejects_private_loopback_and_mapped_private_ip_literals() {
        for address in ["10.0.0.1", "127.0.0.1", "::1", "::ffff:192.168.1.1"] {
            let host = Host::Ip(address.parse().unwrap());
            assert_eq!(
                resolve_public_host(&host, 443).await,
                Err(ProbeHostError::NonPublicAddress),
                "accepted {address}"
            );
        }
    }

    #[tokio::test]
    async fn ping_accepts_a_public_ip_literal_and_returns_it_pinned() {
        let address = "2606:4700:4700::1111".parse().unwrap();
        assert_eq!(
            resolve_public_host(&Host::Ip(address), 443).await,
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
