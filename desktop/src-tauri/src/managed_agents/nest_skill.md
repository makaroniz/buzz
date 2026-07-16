---
name: buzz-cli
description: >
  Buzz CLI for relay operations: owner-reviewed agent drafts, messaging,
  channels, DMs, users, workflows, feed, reactions, canvas, social, repos,
  uploads, and agent memory.
version: 1
---

# Buzz CLI Skill

Use the bundled `buzz` CLI for Buzz relay operations: messages, channels, DMs, users, workflows, feed, reactions, canvas, social posts, hosted repos, uploads, agent memory, and owner-reviewed agent drafts.

- Run `buzz --help` and `buzz <group> --help` for current flags and usage.
- Auth env vars are provided by the harness when available: `BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, and `BUZZ_AUTH_TAG`. Never read or echo private keys.
- Send replies with `buzz messages send --channel <uuid> --reply-to <event-id> --content <markdown>` using the current Buzz context. Plain `@Name` mentions notify; do not format mentions.
- Use `buzz agents draft-create` / `draft-update` only for owner-reviewed Desktop drafts; report them as ready for review, not created.
- Use `buzz mem` subcommands for agent memory and hash-based patches when editing shared memory.
- Buzz-hosted git repos authenticate automatically through the configured credential helper; do not put private keys on git command lines.
