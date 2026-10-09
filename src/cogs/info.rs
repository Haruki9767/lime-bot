use std::time::Duration;

use crate::{Data, Error};
use lime_discord_bot::utils::format_uptime;

/// Export this module's slash-command list for composition in `cogs::commands()`.
pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![uptime(), help(), github()]
}

const HELP_TEXT: &str = const HELP_TEXT: &str = "\
**Lime Bot Commands**

**`/curl`** `url:<URL>` `[include_headers]` `[head_only]` `[silent]` `[follow_redirects]` `[output_file:<name>]`
> Safe HTTP(S) GET/HEAD requests.
> • Redirects are **off** by default; `follow_redirects` follows up to 5 checked hops
> • Inline output capped at **32 KiB**, file attachments at **5 MiB**

**`/ping`** `host:<public host>` `[port:<port>]`
> Tests reachability — ICMP where available, otherwise TCP.
> • Port defaults to `443`

**`/dns`** `domain:<domain>` `[type:<A|AAAA|MX|TXT|NS|CNAME>]`
> Looks up DNS records.
> • Type defaults to `A`

**`/whois`** `domain:<domain>`
> Looks up registration details and nameservers.

**`/uptime`**
> Shows how long the bot process has been running.

**`/github`**
> Links to the bot source code.

**`/help`**
> Shows this command guide.";

/// Explain the bot's available slash commands.
#[poise::command(slash_command)]
async fn help(ctx: poise::Context<'_, Data, Error>) -> Result<(), Error> {
    ctx.say(HELP_TEXT).await?;
    Ok(())
}

/// Link to the bot source code on the lime branch.
#[poise::command(slash_command)]
async fn github(ctx: poise::Context<'_, Data, Error>) -> Result<(), Error> {
    ctx.say("Lime Bot source code (lime branch): https://github.com/Haruki9767/lime-bot/tree/lime")
        .await?;
    Ok(())
}

#[poise::command(slash_command)]
async fn uptime(ctx: poise::Context<'_, Data, Error>) -> Result<(), Error> {
    let uptime = format_uptime(ctx.data().start_time.elapsed());
    // Poise's Serenity context exposes a shard messenger and ID, not the shard
    // runner's heartbeat measurement. Report this explicitly instead of guessing.
    let latency = format_gateway_latency(None);

    ctx.say(format!("Uptime: {uptime}\n{latency}")).await?;
    Ok(())
}

fn format_gateway_latency(latency: Option<Duration>) -> String {
    match latency {
        Some(latency) => format!("Gateway heartbeat latency: {} ms", latency.as_millis()),
        None => "Gateway heartbeat latency: unavailable".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lime_discord_bot::utils::MAX_DISCORD_MESSAGE_CHARS;

    #[test]
    fn help_text_fits_within_discord_message_limit() {
        assert!(HELP_TEXT.encode_utf16().count() <= MAX_DISCORD_MESSAGE_CHARS);
    }

    #[test]
    fn latency_is_reported_only_when_a_shard_has_a_measurement() {
        assert_eq!(
            format_gateway_latency(Some(Duration::from_millis(42))),
            "Gateway heartbeat latency: 42 ms"
        );
        assert_eq!(
            format_gateway_latency(None),
            "Gateway heartbeat latency: unavailable"
        );
    }
}
