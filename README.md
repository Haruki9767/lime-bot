# Lime Discord Bot

Lime is a modular Rust 2024 Discord bot using Tokio, Serenity 0.12, and Poise 0.6. It supports **slash commands only**—there are no text-prefix handlers and no Message Content intent requirement.

## Requirements and configuration

- Rust **1.85 or newer** (edition 2024)
- A Discord application with a bot user and token
- For Wispbyte, a Linux container compatible with the generated executable and the outbound network requirements below

### Discord application setup

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications) and add a bot user.
2. Invite it using the `bot` and `applications.commands` OAuth scopes. Grant the bot permission to send messages where users will run commands.
3. Do not enable privileged Gateway intents; this bot uses application commands only.
4. For local work, copy `.env.example` to `.env` and set `DISCORD_TOKEN`. Never commit a real token or put it in source code.
5. Optionally set `DISCORD_GUILD_ID` to a development server’s numeric ID for fast guild-scoped registration. Omit it for global deployment; global command changes may take time to propagate.

## Run and verify locally

```sh
cp .env.example .env
# Edit .env and set DISCORD_TOKEN; optionally set DISCORD_GUILD_ID.
cargo run --locked
```

Run the project checks and build a release binary with:

```sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

The bot’s memory, network operations, request/response bodies, DNS result count, and Discord output are bounded for small containers. It uses `rustls`, not native TLS, and does not rely on system `curl`, `whois`, or `dig` binaries. The optional Linux `ping` executable is the only external command it attempts.

## Slash commands

- **`/curl url:<URL>`** — performs a bounded HTTP **GET**. It accepts only HTTP/HTTPS, rejects credentials and non-public IPv4/IPv6 destinations (including mixed public/private DNS answers), pins the validated DNS results to prevent rebinding, disables redirects, limits request time and body size, and escapes/truncates the result safely for Discord.
- **`/ping host:<host> [port:<port>]`** — accepts only public Internet destinations. It resolves hostnames once, rejects the full answer set if any address is non-public, and probes a checked IP. On Linux it attempts ICMP with a process timeout if the `ping` executable can be started; if the executable is missing or cannot be launched, it uses a bounded TCP connectivity check to the supplied port (default `443`). TCP fallback is labeled as TCP, not ICMP. Use only for destinations you are authorized to test; cooldowns are applied per user.
- **`/whois domain:<domain>`** — validates a public domain, queries IANA then follows at most one validated registry referral over TCP port 43, and extracts registrar, creation date, expiry date, and nameservers best-effort. Missing fields are reported as `not provided`; registry output varies. A host or network that blocks outbound TCP/43 can prevent results.
- **`/dns domain:<domain> [type:<type>]`** — queries A, AAAA, MX, TXT, NS, or CNAME records using the configured system resolver; type defaults to A. Query time, result count, and Discord output are bounded. No records and resolver failures receive clear responses.
- **`/uptime`** — reports process uptime. The Poise command context does not expose the shard runner’s heartbeat measurement, so gateway latency is explicitly reported as unavailable rather than invented.

Network commands defer their Discord response before slow I/O, so DNS, TCP, or HTTP waits do not miss Discord’s initial interaction-response deadline ([Discord interaction response docs](https://discord.com/developers/docs/interactions/receiving-and-responding)).

### How `/curl` can gain more options

Today `/curl` takes one URL and always performs a GET. It is implemented with the Rust `reqwest` library—not by shelling out to the operating-system `curl` program—so arbitrary CLI flags are not accepted. Query strings already work as part of the URL. To add behavior later, add explicit slash-command parameters such as a method or a small set of allowed headers, then validate each option and keep the existing destination checks, timeout, redirect policy, body limits, mention/code-block escaping, and cooldowns. Do not pass user text to a shell. Avoid allowing credentials or unrestricted headers, since those can leak secrets or enable abuse against public endpoints.

## Network requirements

The container needs outbound connectivity for:

- Discord’s HTTPS API and Gateway WebSocket over TLS (normally outbound port 443)
- DNS resolution through the container’s configured resolver
- HTTP/HTTPS to public hosts for `/curl`
- TCP port 43 for `/whois` (may be blocked by hosting providers or registry policy)
- Optional ICMP for `/ping`; otherwise the command may use TCP to the selected target port

The bot does **not** listen on an inbound HTTP port. Hosting firewalls and sandbox capabilities may restrict outbound connections or ICMP.

## Deploy on Wispbyte

Wispbyte’s public [Startup Settings](https://wispbyte.com/kb/startup-settings) and [GitHub Integration](https://wispbyte.com/kb/github-integration) documentation describes panel-managed images, startup commands, environment variables, and repository sync. I could not verify a Wispbyte-documented Rust build image/toolchain. Therefore, the reliable route is to build a compatible Linux binary first and upload it. If the panel offers a Rust-capable image, you can instead clone this branch and build there, but confirm that Rust 1.85+ and Cargo are available; do not assume this.

### 1. Build the executable

On a compatible Linux **x86_64** machine, from the repository root:

```sh
cargo build --release --locked
file target/release/lime-discord-bot
ldd target/release/lime-discord-bot
```

The default target is an x86_64 Linux executable linked to glibc. Choose a Wispbyte container with a matching CPU architecture and compatible runtime libraries. Do not upload a Windows or macOS binary. Rust and Cargo are unnecessary in the Wispbyte container when using the prebuilt binary.

### 2. Set up the Wispbyte server

1. Create a bot/app server in the [Wispbyte client panel](https://wispbyte.com/client).
2. Select a Linux container that allows a custom shell startup command, executable uploads, outbound Discord connectivity, and the binary’s CPU/ABI. If no available image meets these conditions, ask Wispbyte support which image can run a prebuilt Linux executable.
3. In **Files**, upload `target/release/lime-discord-bot` into the server’s working directory (usually the root). Wispbyte’s GitHub integration can clone/pull source, but that alone does not compile Rust.
4. In **Startup**, set `DISCORD_TOKEN` to the bot token. Never put the token in the repository or startup command. Add `DISCORD_GUILD_ID` only for guild-scoped development registration; omit it for global registration.
5. Set the **Startup Command** to:

   ```sh
   chmod +x ./lime-discord-bot && ./lime-discord-bot
   ```

6. Save settings and start the server from **Console**. Read the logs if it exits; confirm the bot is online and test `/uptime`, `/dns`, and `/ping`.

The executable approach depends on the actual Wispbyte image and network policy, which cannot be tested from this repository environment. Check Wispbyte’s current per-server bot and resource limits in the panel.

## Add another slash-command group

1. Add a module such as `src/cogs/example.rs` and define slash commands there using `#[poise::command(slash_command)]`.
2. Export `pub fn commands() -> Vec<poise::Command<crate::Data, crate::Error>>` from the module.
3. Declare the module in `src/cogs/mod.rs` and append its command list in `cogs::commands()`.
4. Put genuinely shared validation/formatting helpers in `src/utils/` and add tests for input boundaries, output limits, and network error cases.

Keep command registration slash-only; do not add prefix/message handlers or the Message Content intent.
