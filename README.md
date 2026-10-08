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

### Runtime diagnostics

When a command encounters a DNS, HTTP, ICMP, TCP, or WHOIS failure, the bot writes a diagnostic line to **stderr** with a `[lime][command]` prefix. Check the terminal running `cargo run` or the hosting panel's live console/logs. Diagnostics include the failing stage and underlying resolver or I/O error where available; they do not include the bot token, full curl URL, URL path/query, or HTTP response body. Hostnames and public WHOIS target addresses may appear, so redact them before sharing logs publicly.

The bot’s memory, network operations, request/response bodies, DNS result count, and Discord output are bounded for small containers. It uses `rustls`, not native TLS, and does not rely on system `curl`, `whois`, or `dig` binaries. The optional Linux `ping` executable is the only external command it attempts.

## Slash commands

- **`/curl url:<URL> [include_headers:true] [head_only:true] [silent:true] [follow_redirects:true] [output_file:<name>]`** — performs a bounded HTTP **GET**, or a **HEAD** request when `head_only` is true. `include_headers:true` includes response headers and the body (like curl `-i`); `head_only:true` sends HEAD and includes response headers without a body (like curl `-I`). `silent:true` omits status/timing metadata but still returns the response and reports errors (like `-s` in a bot without a progress meter). `follow_redirects:true` follows up to five redirects (like `-L`), validating and DNS-pinning every destination; redirects are off by default. `output_file:<name>` attaches the response using a sanitized filename for download (like `-o`); Discord lets you choose where to save it. Inline response data is capped at 32 KiB; file attachments at 5 MiB. Oversized responses are truncated with a notice. Sensitive response-header values such as `Set-Cookie` are redacted.
- **`/ping host:<host> [port:<port>]`** — accepts only public Internet destinations. It resolves hostnames once, rejects the full answer set if any address is non-public, and probes a checked IP. On Linux it attempts ICMP with a process timeout if the `ping` executable can be started; if the executable is missing or cannot be launched, it uses a bounded TCP connectivity check to the supplied port (default `443`). TCP fallback is labeled as TCP, not ICMP. Use only for destinations you are authorized to test; cooldowns are applied per user.
- **`/whois domain:<domain>`** — validates a public domain, queries IANA then follows at most one validated registry referral over TCP port 43, and extracts registrar, creation date, expiry date, and nameservers best-effort. Missing fields are reported as `not provided`; registry output varies. A host or network that blocks outbound TCP/43 can prevent results.
- **`/dns domain:<domain> [type:<type>]`** — queries A, AAAA, MX, TXT, NS, or CNAME records using the configured system resolver; type defaults to A. Query time, result count, and Discord output are bounded. No records and resolver failures receive clear responses.
- **`/uptime`** — reports process uptime. The Poise command context does not expose the shard runner’s heartbeat measurement, so gateway latency is explicitly reported as unavailable rather than invented.
- **`/help`** — explains every slash command and its main options.
- **`/github`** — links to the bot source code on the `lime` branch.

Network commands defer their Discord response before slow I/O, so DNS, TCP, or HTTP waits do not miss Discord’s initial interaction-response deadline ([Discord interaction response docs](https://discord.com/developers/docs/interactions/receiving-and-responding)).

### `/curl` options and limitations

`/curl` is implemented with Rust's `reqwest` library, not by shelling out to the operating-system `curl` program, so enter the named slash-command options rather than raw flags. Quiet mode suppresses status/timing details, not errors or the required Discord response. Redirects are manually followed only when requested and each target is revalidated; do not enable unrestricted automatic redirects or pass user text to a shell.

Examples: use `/curl url:https://example.com include_headers:true` for headers plus body; `/curl url:https://example.com head_only:true` for headers only; `/curl url:https://example.com silent:true` to omit status/timing; `/curl url:https://example.com follow_redirects:true output_file:page.html` to safely follow redirects and receive a downloadable attachment named `page.html`.

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
