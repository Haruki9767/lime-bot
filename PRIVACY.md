# Lime Bot Privacy Policy

**Effective date: October 9, 2026**

This Privacy Policy explains how Lime Bot (the “Bot”) handles information when you invite it to a Discord server or use its slash commands. The Bot’s source code is available in this repository. This policy describes the behavior of the current `lime` branch and may be updated when the Bot changes.

## Information the Bot processes

When you use the Bot, Discord sends the Bot the interaction data needed to run the requested slash command. Depending on the command, this can include your Discord user ID, server and channel context supplied by Discord, the command name, and the values you enter, such as a URL, hostname, port, domain, DNS record type, or output filename.

The Bot keeps a small in-memory cooldown map keyed by Discord user ID and command name. These entries are automatically removed after approximately 60 seconds and are not written to a database by this repository. The Bot also keeps its process start time in memory for `/uptime` and its presence display.

Network commands may send the values needed to perform the requested operation to external network services. For example, `/curl` contacts the public HTTP or HTTPS host you provide, `/dns` uses the container’s configured DNS resolver, and `/whois` contacts IANA and, when returned by IANA, one validated registry referral over TCP port 43. `/ping` resolves and probes the public host and port you provide. The Bot does not intentionally send the Discord bot token as part of these operations.

## Logs and retention

The Bot writes operational diagnostics to standard error when network or command operations fail. Depending on the command, diagnostics can include hostnames, domains, public WHOIS target addresses, failure stages, and resolver or I/O errors. The Bot avoids logging the bot token, full `/curl` URLs, URL paths and queries, and HTTP response bodies.

This repository does not implement a persistent application database, analytics tracker, advertising profile, or sale of personal information. In-memory cooldown data ends when it expires or the process stops. Discord, the hosting provider, DNS infrastructure, and any external host contacted by a command may retain information under their own policies and operational logs. The repository cannot control those third-party retention periods.

## Discord and third-party services

The Bot depends on Discord’s API and Gateway to receive interactions and send responses. Command behavior can also involve the public hosts and registry or DNS services described above. Their processing is governed by their own terms and privacy policies. The Bot is designed to use non-privileged Discord intents and does not require the Message Content intent for its slash-command operation.

## Your choices and requests

You can stop the Bot from processing new commands by not using it, removing it from a server, or asking a server administrator to remove it. You can request clarification about this policy or report a privacy or security concern by opening an issue in the [Lime Bot repository](https://github.com/Haruki9767/lime-bot/issues). Do not include tokens, passwords, private URLs, or other sensitive information in a public issue.

Requests involving Discord account data or third-party service logs may need to be directed to the relevant service because this repository does not control those systems.

## Changes

When the Bot’s data handling materially changes, this document will be updated with a new effective date. Continued use of the Bot after an update means you acknowledge the revised policy.
