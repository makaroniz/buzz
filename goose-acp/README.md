# goose-acp — bundled slim goose agent

A minimal binary exposing [goose](https://github.com/aaif-goose/goose) as an
ACP agent server on stdio, so the Buzz distribution can bundle a capable
goose agent without users installing goose themselves.

## What's in / what's out

Built from the `goose` library crate with `default-features = false`,
`features = ["rustls-tls"]`:

| Included | Excluded |
|---|---|
| Full agent loop, sessions, compaction | code-mode (pctx → V8 JS/TS runtime) |
| All API providers (Anthropic, OpenAI, …) | local-inference (llama.cpp/candle) |
| MCP extensions, developer builtin | system keyring, AWS providers |
| ACP stdio server | telemetry/otel, goose CLI/TUI |

Result: ~32 MB on macOS arm64 (vs ~240 MB for a full goose install), adding
~9 MB to the compressed desktop download. When goose's `analyze`/`tiktoken`
feature gates land upstream (branch `micn/slim-features`), bumping the pinned
rev drops this to ~14 MB raw / ~5 MB compressed with no changes here.

## Why a separate workspace

This crate is excluded from the Buzz workspace on purpose:

- goose's ~700-crate dependency graph stays out of the workspace Cargo.lock
- goose requires **rmcp 1.7.x** (1.8 breaks its build). The pin lives in this
  crate's own lockfile. If you ever regenerate it:
  `cargo update rmcp --precise 1.7.0`

## Building

```bash
cargo build --release --manifest-path goose-acp/Cargo.toml
# → goose-acp/target/release/goose-acp
```

`scripts/bundle-sidecars.sh` stages it as a Tauri sidecar
(`binaries/goose-acp-<triple>`) when present. Release CI builds it
explicitly; local dev flows that skip it still work (the desktop app treats
a missing/stub binary as "not installed" and falls back to `goose` on PATH
or `buzz-agent`).

## Using it

The binary **is** the ACP server — no `acp` subcommand (buzz-acp knows this;
see `default_agent_args` in `crates/buzz-acp/src/config.rs`):

```bash
BUZZ_ACP_AGENT_COMMAND=/path/to/goose-acp buzz-acp ...
```

Provider/model configuration is standard goose: `GOOSE_PROVIDER`,
`GOOSE_MODEL`, provider API-key env vars, or `~/.config/goose/config.yaml`.
