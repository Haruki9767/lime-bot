use std::{net::SocketAddr, time::Duration};

use hickory_resolver::proto::rr::RecordType;
use poise::serenity_prelude::UserId;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
    time::timeout,
};

use crate::{CooldownCheck, Data, Error};
use lime_discord_bot::utils::{format_code_block, is_public_ip, validate_domain};

const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_DNS_RECORDS: usize = 12;
const MAX_DNS_RECORD_CHARS: usize = 320;

const WHOIS_COOLDOWN: Duration = Duration::from_secs(15);
const WHOIS_DNS_TIMEOUT: Duration = Duration::from_secs(5);
const WHOIS_IO_TIMEOUT: Duration = Duration::from_secs(8);
const WHOIS_RESPONSE_LIMIT: usize = 16 * 1024;
const WHOIS_MAX_LINES: usize = 512;

/// Export this module's slash-command list for composition in `cogs::commands()`.
pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![whois(), dns()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedRecordType {
    A,
    Aaaa,
    Mx,
    Txt,
    Ns,
    Cname,
}

impl SupportedRecordType {
    fn parse(input: Option<&str>) -> Option<Self> {
        match input.unwrap_or("A").trim().to_ascii_uppercase().as_str() {
            "A" => Some(Self::A),
            "AAAA" => Some(Self::Aaaa),
            "MX" => Some(Self::Mx),
            "TXT" => Some(Self::Txt),
            "NS" => Some(Self::Ns),
            "CNAME" => Some(Self::Cname),
            _ => None,
        }
    }

    fn record_type(self) -> RecordType {
        match self {
            Self::A => RecordType::A,
            Self::Aaaa => RecordType::AAAA,
            Self::Mx => RecordType::MX,
            Self::Txt => RecordType::TXT,
            Self::Ns => RecordType::NS,
            Self::Cname => RecordType::CNAME,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Mx => "MX",
            Self::Txt => "TXT",
            Self::Ns => "NS",
            Self::Cname => "CNAME",
        }
    }
}

#[poise::command(slash_command)]
async fn dns(
    ctx: poise::Context<'_, Data, Error>,
    domain: String,
    r#type: Option<String>,
) -> Result<(), Error> {
    let domain = match validate_domain(&domain) {
        Ok(domain) => domain,
        Err(_) => {
            ctx.say("Enter a valid public DNS domain name.").await?;
            return Ok(());
        }
    };
    let Some(record_type) = SupportedRecordType::parse(r#type.as_deref()) else {
        ctx.say("Unsupported DNS type. Choose A, AAAA, MX, TXT, NS, or CNAME.")
            .await?;
        return Ok(());
    };

    if let Some(message) = cooldown_message(ctx.data(), ctx.author().id, "dns", DNS_COOLDOWN) {
        ctx.say(message).await?;
        return Ok(());
    }

    ctx.defer().await?;
    let query = timeout(
        DNS_TIMEOUT,
        ctx.data()
            .resolver
            .lookup(domain.as_str(), record_type.record_type()),
    )
    .await;

    match query {
        Ok(Ok(lookup)) => {
            let rows: Vec<String> = lookup
                .iter()
                .take(MAX_DNS_RECORDS)
                .map(|record| {
                    record
                        .to_string()
                        .chars()
                        .take(MAX_DNS_RECORD_CHARS)
                        .collect()
                })
                .collect();
            let content = format_dns_records(&rows, lookup.records().len());
            ctx.say(format_code_block(
                &format!("DNS {} {}", record_type.label(), domain),
                &content,
            ))
            .await?;
        }
        Ok(Err(_)) => {
            ctx.say(format_code_block(
                &format!("DNS {} {}", record_type.label(), domain),
                "The resolver returned an error. Check DNS connectivity and try again.",
            ))
            .await?;
        }
        Err(_) => {
            ctx.say(format_code_block(
                &format!("DNS {} {}", record_type.label(), domain),
                "The DNS query timed out.",
            ))
            .await?;
        }
    }
    Ok(())
}

fn format_dns_records(rows: &[String], total: usize) -> String {
    if total == 0 || rows.is_empty() {
        return "No records found.".to_owned();
    }
    let mut output = rows.join("\n");
    if total > rows.len() {
        output.push_str(&format!("\n… and {} more record(s)", total - rows.len()));
    }
    output
}

#[poise::command(slash_command)]
async fn whois(ctx: poise::Context<'_, Data, Error>, domain: String) -> Result<(), Error> {
    let domain = match validate_domain(&domain) {
        Ok(domain) => domain,
        Err(_) => {
            ctx.say("Enter a valid public DNS domain name.").await?;
            return Ok(());
        }
    };

    if let Some(message) = cooldown_message(ctx.data(), ctx.author().id, "whois", WHOIS_COOLDOWN) {
        ctx.say(message).await?;
        return Ok(());
    }

    ctx.defer().await?;
    match query_whois(&domain).await {
        Ok(response) => {
            let fields = parse_whois_fields(&response);
            let content = format_whois_fields(&fields);
            ctx.say(format_code_block(&format!("WHOIS {domain}"), &content))
                .await?;
        }
        Err(error) => {
            ctx.say(format_code_block(
                &format!("WHOIS {domain}"),
                whois_error_message(error),
            ))
            .await?;
        }
    }
    Ok(())
}

fn cooldown_message(
    data: &Data,
    user_id: UserId,
    command: &'static str,
    duration: Duration,
) -> Option<String> {
    match data.check_cooldown(user_id.get(), command, duration) {
        CooldownCheck::Allowed => None,
        CooldownCheck::Wait(remaining) => Some(format!(
            "Please wait {:.1} seconds before using this command again.",
            remaining.as_secs_f32()
        )),
        CooldownCheck::AtCapacity => Some("The bot is busy; try again shortly.".to_owned()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhoisError {
    InvalidServer,
    ResolutionFailed,
    NoAddresses,
    NonPublicAddress,
    Io,
    Timeout,
    ResponseTooLarge,
}

async fn query_whois(domain: &str) -> Result<String, WhoisError> {
    let iana_response = whois_query("whois.iana.org", domain).await?;
    let Some(referral) = find_referral(&iana_response) else {
        return Ok(iana_response);
    };
    if referral == "whois.iana.org" {
        return Ok(iana_response);
    }
    whois_query(&referral, domain).await
}

async fn whois_query(server: &str, domain: &str) -> Result<String, WhoisError> {
    let server = validate_domain(server).map_err(|_| WhoisError::InvalidServer)?;
    let resolved = timeout(WHOIS_DNS_TIMEOUT, lookup_host((server.as_str(), 43)))
        .await
        .map_err(|_| WhoisError::ResolutionFailed)?
        .map_err(|_| WhoisError::ResolutionFailed)?;
    let addresses: Vec<SocketAddr> = resolved.collect();
    if addresses.is_empty() {
        return Err(WhoisError::NoAddresses);
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(WhoisError::NonPublicAddress);
    }
    let address = addresses[0];
    let request = format!("{domain}\r\n");

    let operation = async move {
        let mut stream = TcpStream::connect(address)
            .await
            .map_err(|_| WhoisError::Io)?;
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| WhoisError::Io)?;
        let mut bytes = Vec::with_capacity(WHOIS_RESPONSE_LIMIT.min(4096));
        let mut reader = stream.take((WHOIS_RESPONSE_LIMIT + 1) as u64);
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| WhoisError::Io)?;
        if bytes.len() > WHOIS_RESPONSE_LIMIT {
            return Err(WhoisError::ResponseTooLarge);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };

    timeout(WHOIS_IO_TIMEOUT, operation)
        .await
        .map_err(|_| WhoisError::Timeout)?
}

fn find_referral(response: &str) -> Option<String> {
    let mut refer = None;
    let mut whois = None;
    for line in response.lines().take(WHOIS_MAX_LINES) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches('.');
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "refer" => {
                refer = validate_domain(value).ok();
            }
            "whois" | "whois server" => {
                whois = validate_domain(value).ok();
            }
            _ => {}
        }
    }
    refer.or(whois)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WhoisFields {
    registrar: Option<String>,
    created: Option<String>,
    expires: Option<String>,
    nameservers: Vec<String>,
}

fn parse_whois_fields(response: &str) -> WhoisFields {
    let mut fields = WhoisFields::default();
    for line in response.lines().take(WHOIS_MAX_LINES) {
        if line.len() > 2_048 {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key: String = key
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let value = clip_field(value.trim(), 200);
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "registrar" | "registrarname" | "sponsoringregistrar" => {
                fields.registrar.get_or_insert(value);
            }
            "creationdate" | "created" | "createdon" | "createddate" | "registeredon"
            | "registrationdate" | "creationtime" => {
                fields.created.get_or_insert(value);
            }
            "registryexpirydate"
            | "registrarregistrationexpirationdate"
            | "expirationdate"
            | "expirydate"
            | "expires"
            | "expireson"
            | "paidtill" => {
                fields.expires.get_or_insert(value);
            }
            "nameserver" | "nameservers" | "nserver" if fields.nameservers.len() < 8 => {
                fields.nameservers.push(value);
            }
            _ => {}
        }
    }
    fields
}

fn clip_field(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn format_whois_fields(fields: &WhoisFields) -> String {
    let registrar = fields.registrar.as_deref().unwrap_or("not provided");
    let created = fields.created.as_deref().unwrap_or("not provided");
    let expires = fields.expires.as_deref().unwrap_or("not provided");
    let nameservers = if fields.nameservers.is_empty() {
        "not provided".to_owned()
    } else {
        fields.nameservers.join(", ")
    };
    format!(
        "Registrar: {registrar}\nCreated: {created}\nExpires: {expires}\nName servers: {nameservers}"
    )
}

fn whois_error_message(error: WhoisError) -> &'static str {
    match error {
        WhoisError::InvalidServer | WhoisError::ResolutionFailed | WhoisError::NoAddresses => {
            "The registry host could not be resolved safely."
        }
        WhoisError::NonPublicAddress => {
            "The registry resolved to a non-public network address; the lookup was blocked."
        }
        WhoisError::Io => {
            "The WHOIS registry did not respond. Outbound TCP port 43 may be blocked."
        }
        WhoisError::Timeout => "The WHOIS request timed out. Outbound TCP port 43 may be blocked.",
        WhoisError::ResponseTooLarge => "The WHOIS response exceeded the size limit.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lime_discord_bot::utils::MAX_DISCORD_MESSAGE_CHARS;

    #[test]
    fn supports_exactly_the_requested_dns_record_types_and_defaults_to_a() {
        assert_eq!(
            SupportedRecordType::parse(None),
            Some(SupportedRecordType::A)
        );
        for (name, expected) in [
            ("A", SupportedRecordType::A),
            ("AAAA", SupportedRecordType::Aaaa),
            ("MX", SupportedRecordType::Mx),
            ("TXT", SupportedRecordType::Txt),
            ("NS", SupportedRecordType::Ns),
            ("CNAME", SupportedRecordType::Cname),
            ("txt", SupportedRecordType::Txt),
        ] {
            assert_eq!(SupportedRecordType::parse(Some(name)), Some(expected));
        }
        assert_eq!(SupportedRecordType::parse(Some("PTR")), None);
        assert_eq!(SupportedRecordType::parse(Some("A;id")), None);
    }

    #[test]
    fn dns_output_limits_record_count_and_handles_empty_results() {
        assert_eq!(format_dns_records(&[], 0), "No records found.");
        let rows: Vec<_> = (0..MAX_DNS_RECORDS)
            .map(|index| format!("record-{index}"))
            .collect();
        let output = format_dns_records(&rows, MAX_DNS_RECORDS + 3);
        assert!(output.contains("… and 3 more record(s)"));
        let message = format_code_block("DNS", &output);
        assert!(message.encode_utf16().count() <= MAX_DISCORD_MESSAGE_CHARS);
    }

    #[test]
    fn whois_parser_extracts_common_field_variants_and_multiple_nameservers() {
        let fields = parse_whois_fields(
            "Registrar: Example Registrar\nCreated On: 2020-01-02\nRegistry Expiry Date: 2030-01-02\nName Server: ns1.example.net\nnserver: ns2.example.net\n",
        );
        assert_eq!(fields.registrar.as_deref(), Some("Example Registrar"));
        assert_eq!(fields.created.as_deref(), Some("2020-01-02"));
        assert_eq!(fields.expires.as_deref(), Some("2030-01-02"));
        assert_eq!(fields.nameservers, ["ns1.example.net", "ns2.example.net"]);
    }

    #[test]
    fn whois_missing_fields_are_reported_as_not_provided() {
        let fields = parse_whois_fields("Domain Name: sample.net\nStatus: active\n");
        let formatted = format_whois_fields(&fields);
        assert!(formatted.contains("Registrar: not provided"));
        assert!(formatted.contains("Created: not provided"));
        assert!(formatted.contains("Expires: not provided"));
        assert!(formatted.contains("Name servers: not provided"));
    }

    #[test]
    fn referral_host_is_validated_before_it_can_be_used() {
        assert_eq!(
            find_referral("domain: sample.net\nrefer: whois.registry.net\n"),
            Some("whois.registry.net".to_owned())
        );
        assert_eq!(find_referral("refer: 127.0.0.1\n"), None);
        assert_eq!(find_referral("refer: whois.registry.net;id\n"), None);
    }
}
