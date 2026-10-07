# Lime Discord Bot

A Rust 2024 Discord bot built with Tokio, Serenity, and Poise. It currently exposes two slash commands: `/curl` and `/ping`.

## Requirements

- Rust **1.85 or newer** (the project uses edition 2024)
- A Discord application and bot token
- For Wispbyte, a Linux server/container compatible with the uploaded executable

## Configure the Discord application

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications) and add a bot user.
2. Invite it to your server with the `bot` and `applications.commands` OAuth scopes. Grant the bot permission to send messages in channels where you will run its commands.
3. No privileged Gateway intents or Message Content intent are needed; this project uses slash commands only.
4. Copy `.env.example` to `.env` for local development and set `DISCORD_TOKEN` to the bot token. Keep the real token secret and never commit `.env`.
5. Optionally set `DISCORD_GUILD_ID` to a test server's numeric ID for fast guild-scoped command registration. Leave it unset for global registration; global command changes may take time to propagate.

## Run locally

```sh
cp .env.example .env
# Edit .env and set DISCORD_TOKEN (and optionally DISCORD_GUILD_ID).
cargo run --locked
```

Run checks before deploying:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

The bot needs outbound connectivity to Discord's Gateway/API. `/curl` also needs DNS and outbound HTTP(S); `/ping` attempts ICMP on Linux when the `ping` executable is available and otherwise falls back to a bounded TCP connection. Hosting-provider firewall rules can restrict these features.

## Deploy on Wispbyte

Wispbyte's [Startup Settings](https://wispbyte.com/kb/startup-settings) document panel-managed startup commands, Docker images, and environment variables. I could not find a Wispbyte-documented Rust build image or Rust-specific build pipeline, so the most portable approach is to compile a Linux executable first and upload it. A source clone alone does not compile the Rust project.

### 1. Build a Linux executable

On a Linux **x86_64** machine with Rust installed, in the repository root:

```sh
cargo build --release --locked
file target/release/lime-discord-bot
ldd target/release/lime-discord-bot
```

Use an executable compatible with the Wispbyte container's CPU architecture and C library (this build normally targets x86_64 Linux with glibc). Do not upload a macOS or Windows build. If you are building from another OS, build inside a compatible Linux environment or cross-compile for the exact target. Rust and Cargo are not needed in the runtime container when using the prebuilt executable.

### 2. Create and configure the Wispbyte server

1. Create a bot/app server in the [Wispbyte client panel](https://wispbyte.com/client).
2. Choose a Linux Docker image/container that can run a custom shell startup command and is compatible with the executable you built. The runtime image must support outbound connections to Discord. If the panel does not offer a compatible runtime or rejects uploaded executables, check with Wispbyte support about a Rust-compatible/custom image before proceeding.
3. In **Files**, upload `target/release/lime-discord-bot` to the server's working directory (usually the server root). You can also use Wispbyte's [GitHub Integration](https://wispbyte.com/kb/github-integration) to clone/pull the source, but the source still needs a Rust toolchain/build step; uploading the prebuilt executable avoids that assumption.
4. In **Startup**, add the environment variable `DISCORD_TOKEN` and set its value to your bot token. Do not put the token in a source file, repository, or startup command. Add `DISCORD_GUILD_ID` only if you want guild-scoped registration for a test server; omit it for global commands.
5. Set the **Startup Command** to:

   ```sh
   chmod +x ./lime-discord-bot && ./lime-discord-bot
   ```

6. Save the settings and start the server from **Console**. Check the console/logs for startup errors, then confirm the bot is online and test `/ping` or `/curl` in Discord.

Wispbyte's panel controls for Docker images can vary by server type. The executable approach is conditional on the chosen container allowing the shell command and having a compatible Linux ABI; verify this with the panel's console on first launch. The bot does not need an inbound listening port.

## Commands and safety

- **`/curl url:<URL>`** — makes a bounded HTTP GET. Only HTTP and HTTPS URLs are allowed; non-public DNS/IP destinations are rejected, redirects are disabled, request time is limited, and the response body/output are capped.
- **`/ping host:<host> [port:<port>]`** — tries Linux ICMP ping when available. If the executable is unavailable (or on non-Linux), it checks TCP connectivity to the requested port (default `443`) instead. A successful ICMP check does not test the supplied TCP port. Use only for destinations you are authorized to test.

Both network commands use per-user cooldowns. Slash-command interactions are acknowledged before network work so slow DNS or connectivity checks do not miss Discord's initial-response deadline.
