# Modular Rust Discord Bot Implementation Plan

> **For agentic workers:** Execute the steps in order in this repository. Steps use checkbox syntax for tracking.

**Goal:** Deliver Lime, a complete, deployable, slash-command-only Rust Discord bot for a constrained Wispbyte Docker container.

**Architecture:** Keep the requested ordinary Rust module/cog-like structure: command groups export `commands()` vectors, while `main.rs` owns configuration, shared state, Poise setup, and command registration. Put reusable input validation, Discord-safe output formatting, uptime formatting, and small security helpers in utilities; use bounded async network operations and test pure logic independently.

**Tech Stack:** Rust 2024 (Rust 1.85+), Tokio, Serenity 0.12, Poise 0.6, reqwest with rustls, Hickory DNS resolver, raw bounded WHOIS TCP, dotenvy for local development, anyhow.

**Spec:** `/home/ubuntu/upload/Prompt__Build_a_modular_Rust_slash-command_Discord.md`

## Global Constraints

- Implement **slash commands only**; no prefix/message command handlers, text prefix, or Message Content Intent.
- Use Rust edition 2024 and require Rust 1.85 or newer.
- Pin mutually compatible dependency versions and verify actual APIs by compiling and testing.
- Never use native TLS; bound network time, response size, output size, and memory use.
- Reject non-public curl destinations and disable redirects unless safe redirect validation is implemented.
- Never depend on root access or optional system tools except as optional capabilities.
- Store the token in `DISCORD_TOKEN`; commit only a placeholder `.env.example`.
- Provide Wispbyte, Discord setup, outbound networking, registration, and extension instructions in README.

## Review Focus

- URL parsing and DNS-resolved private/loopback/link-local/reserved destinations must be rejected before request, including IPv4-mapped IPv6 and redirects.
- User-controlled HTTP bodies, WHOIS fields, DNS records, and errors must not break Discord code blocks, exceed 2,000 characters, or reveal internals.
- Malformed hostnames/domains and unsupported record types must return clear errors without panic.
- Optional ICMP executable absence, subprocess timeout, and untrusted process output must safely fall back to an explicitly labeled bounded TCP check or a helpful error.
- Rate-limited network commands must not create unbounded state or permit unrestricted repeated probes.

---

### Task 1: Scaffold, dependency pins, utilities, and pure-logic tests

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `.env.example`
- Create: `src/utils/mod.rs`, `src/utils/format.rs`, `src/utils/validation.rs`
- Test: unit tests in utility modules

**Interfaces:**
- Produces `validate_domain(&str) -> Result<String, ValidationError>`, `validate_host(&str) -> Result<Host, ValidationError>`, `format_uptime(Duration) -> String`, and bounded Discord/code-block formatting functions for later command modules.
- Select exact compatible dependency releases and features; establish pinned versions and compile them before command work.

- [ ] **Step 1: Add tests** for domain/hostname/IP acceptance and rejection (empty, whitespace, bad labels, shell metacharacters, IPv4/IPv6 literals), uptime unit boundaries, control-character neutralization, backtick neutralization, and whole-message truncation at the configured safe target.
- [ ] **Step 2: Run the focused tests** and record the expected initial missing-module/compiler failure.
- [ ] **Step 3: Implement utilities** with Rust `std::net` parsing and explicit label validation, `Duration`-based uptime formatting, and UTF-8-boundary-safe truncation that accounts for all surrounding Discord markup.
- [ ] **Step 4: Add Cargo configuration** using edition 2024, pinned compatible crate versions, Tokio features, reqwest rustls-only features, Serenity/Poise, Hickory resolver, anyhow, dotenvy, and a release profile with `strip = true` and `opt-level = "z"`; exclude `.env`, `target/`, and local secrets in `.gitignore`.
- [ ] **Step 5: Run `cargo fmt --check` and `cargo test`**; revise until the focused utility tests pass.

### Task 2: Bounded curl and ping commands

**Files:**
- Create: `src/cogs/mod.rs`, `src/cogs/network.rs`
- Modify: `src/utils/validation.rs` and `src/utils/format.rs` if shared test feedback requires it
- Test: command-module pure helper/unit tests

**Interfaces:**
- Exports `pub fn commands() -> Vec<poise::Command<Data, Error>>` from `network`.
- Consumes `Data.http_client`, common error alias, validation, and Discord-safe format helpers from Task 1.

- [ ] **Step 1: Add tests** for allowed/rejected URL schemes, localhost/private/link-local/public IP policies (including IPv4-mapped IPv6), redirect policy, response size edge limits, and output truncation/code-block escaping.
- [ ] **Step 2: Implement `/curl`** with parsed `http`/`https` URLs, DNS address resolution and public-address checks before connecting, a 10-second timeout, a strict body cap, redirect policy `none`, status/elapsed time, and a total response target no greater than 1,800 characters. Prevent rebinding between validation and connection by using a client/connector strategy that pins validated resolution for the request; if the chosen HTTP stack cannot guarantee it, reject hostnames where that guarantee is unavailable rather than claiming safety.
- [ ] **Step 3: Implement `/ping`** using validated host input; on Linux invoke `ping -c 3` directly with arguments (never a shell), bounded output and process timeout, treating absence as optional. If unavailable, TCP-connect to supplied port/default 443 with a short timeout and label it precisely as a TCP connectivity check. Add per-user cooldowns with bounded/expiring state and a clear probing-use disclaimer.
- [ ] **Step 4: Run the network-module unit tests** and correct any failures.

### Task 3: WHOIS and DNS lookup commands

**Files:**
- Create: `src/cogs/lookup.rs`
- Modify: `src/utils/validation.rs` as needed
- Test: lookup parsing and validation unit tests

**Interfaces:**
- Exports `pub fn commands() -> Vec<poise::Command<Data, Error>>` from `lookup`.
- Uses common domain validation, safe output helpers, and bounded rate limiting.

- [ ] **Step 1: Add tests** for valid/invalid domains, WHOIS field extraction under different common label spellings and missing-field behavior, all six record-type choices, record count/size caps, and empty results.
- [ ] **Step 2: Implement `/whois`** as a raw TCP/43 query with bounded connect/read timeouts and strict byte limit; parse registrar, creation date, expiry date, and nameservers best-effort and show missing values as “not provided”. Apply bounded per-user cooldown state.
- [ ] **Step 3: Implement `/dns`** with the verified Hickory resolver API, `A` default, support for `A`, `AAAA`, `MX`, `TXT`, `NS`, `CNAME`, explicit query timeout, response count/size caps, safe code-block formatting, and per-user cooldown.
- [ ] **Step 4: Run the lookup unit tests** and correct any failures.

### Task 4: Shared application state, uptime, and command registration

**Files:**
- Create: `src/main.rs`, `src/cogs/info.rs`
- Modify: `src/cogs/mod.rs`
- Test: uptime formatting tests in utilities

**Interfaces:**
- Defines shared `Data` containing `start_time: Instant`, reusable reqwest client, and only necessary bounded shared rate-limit state.
- Defines an anyhow-compatible command error type and command context aliases consumed by all modules.
- Exports `pub fn commands() -> Vec<poise::Command<Data, Error>>` from `info` and concatenates all command vectors in `main`.

- [ ] **Step 1: Implement `/uptime`** using process `Instant`, rendered as days/hours/minutes/seconds; call only an actually supported Serenity 0.12 shard-latency API if verified, otherwise state latency is unavailable.
- [ ] **Step 2: Configure Poise/Serenity** for application commands only, using `DISCORD_TOKEN`; optionally load `.env` locally; support a guild-ID environment setting for development registration and global registration otherwise; do not enable privileged intents or message-content handling.
- [ ] **Step 3: Run `cargo fmt --check`, `cargo check`, and unit tests** to validate exact framework and resolver API compatibility.

### Task 5: Complete deployment documentation and full verification

**Files:**
- Create: `README.md`
- Modify: project files based on compile/test findings

**Interfaces:**
- README documents local development, Wispbyte Docker deployment, exact environment settings and registration choice, command behavior, Discord install configuration, network/security limitations, and how to add a command group.

- [ ] **Step 1: Write README** explicitly stating slash commands only; explain bot/application setup and minimum installation permissions without Message Content Intent; distinguish development guild registration from global propagation delays; document outbound Discord, DNS, HTTP(S), optional ICMP, and TCP/43 requirements, WHOIS variability, and Wispbyte image/network assumptions.
- [ ] **Step 2: Run verification**: `cargo fmt --check`, `cargo check`, and `cargo test`; capture Rust/compiler and dependency versions. If network/dependency access prevents a command, record the exact failure without claiming it passed.
- [ ] **Step 3: Inspect the finished project** for every required path, registration composition, absence of prefix handlers/Message Content Intent, placeholder-only token, bounded outputs, and absence of real secrets. Report any deployment behavior that cannot be verified in this environment.
