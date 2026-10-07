use std::{
    collections::HashMap,
    env,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context as _, bail};
use poise::serenity_prelude::{GatewayIntents, GuildId};

mod cogs;

const MAX_COOLDOWN_ENTRIES: usize = 4_096;
const COOLDOWN_ENTRY_TTL: Duration = Duration::from_secs(10);

pub type Error = anyhow::Error;

/// Shared bot state. The small cooldown map is bounded and never stores network clients.
pub struct Data {
    pub start_time: Instant,
    cooldowns: Mutex<HashMap<(u64, &'static str), Instant>>,
}

impl Data {
    pub(crate) fn check_cooldown(
        &self,
        user_id: u64,
        command: &'static str,
        cooldown: Duration,
    ) -> CooldownCheck {
        let now = Instant::now();
        let mut entries = self
            .cooldowns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, checked_at| {
            now.saturating_duration_since(*checked_at) < COOLDOWN_ENTRY_TTL
        });

        let key = (user_id, command);
        if let Some(previous) = entries.get_mut(&key) {
            let elapsed = now.saturating_duration_since(*previous);
            if elapsed < cooldown {
                return CooldownCheck::Wait(cooldown - elapsed);
            }
            *previous = now;
            return CooldownCheck::Allowed;
        }

        if entries.len() >= MAX_COOLDOWN_ENTRIES {
            return CooldownCheck::AtCapacity;
        }
        entries.insert(key, now);
        CooldownCheck::Allowed
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CooldownCheck {
    Allowed,
    Wait(Duration),
    AtCapacity,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    {
        let _ = dotenvy::dotenv();
    }

    let token = env::var("DISCORD_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
        .context("DISCORD_TOKEN must be set to a non-empty bot token")?;
    let development_guild = match env::var("DISCORD_GUILD_ID") {
        Ok(value) => Some(
            value
                .parse::<u64>()
                .context("DISCORD_GUILD_ID must be a valid positive integer")
                .and_then(|id| {
                    if id == 0 {
                        bail!("DISCORD_GUILD_ID must be a valid positive integer")
                    } else {
                        Ok(GuildId::new(id))
                    }
                })?,
        ),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            bail!("DISCORD_GUILD_ID must be valid Unicode containing a positive integer")
        }
    };

    let commands = cogs::commands();
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands,
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                if let Some(guild_id) = development_guild {
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        guild_id,
                    )
                    .await?;
                } else {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                }

                Ok(Data {
                    start_time: Instant::now(),
                    cooldowns: Mutex::new(HashMap::new()),
                })
            })
        })
        .build();

    let intents = GatewayIntents::non_privileged();
    let mut client = poise::serenity_prelude::ClientBuilder::new(token, intents)
        .framework(framework)
        .await
        .context("could not initialize the Discord client")?;
    client
        .start()
        .await
        .context("the Discord client stopped with an error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Data {
        Data {
            start_time: Instant::now(),
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn cooldowns_are_per_user_and_per_command() {
        let state = data();
        let delay = Duration::from_secs(10);
        assert_eq!(
            state.check_cooldown(7, "curl", delay),
            CooldownCheck::Allowed
        );
        assert!(matches!(
            state.check_cooldown(7, "curl", delay),
            CooldownCheck::Wait(remaining) if remaining > Duration::ZERO
        ));
        assert_eq!(
            state.check_cooldown(7, "ping", delay),
            CooldownCheck::Allowed
        );
        assert_eq!(
            state.check_cooldown(8, "curl", delay),
            CooldownCheck::Allowed
        );
    }

    #[test]
    fn cooldown_state_fails_closed_at_its_finite_capacity() {
        let state = data();
        for user in 0..MAX_COOLDOWN_ENTRIES as u64 {
            assert_eq!(
                state.check_cooldown(user, "ping", COOLDOWN_ENTRY_TTL),
                CooldownCheck::Allowed
            );
        }
        assert_eq!(
            state.check_cooldown(u64::MAX, "ping", COOLDOWN_ENTRY_TTL),
            CooldownCheck::AtCapacity
        );
    }
}
