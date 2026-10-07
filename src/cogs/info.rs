use std::time::Duration;

use crate::{Data, Error};
use lime_discord_bot::utils::format_uptime;

/// Export this module's slash-command list for composition in `cogs::commands()`.
pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![uptime()]
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
