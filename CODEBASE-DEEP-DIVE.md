# Buzz Codebase Deep-Dive

> Working notes from a full-repo exploration (July 2026, repo at commit `7e34bee6`).
> Compiled by Claude Code from ARCHITECTURE.md plus five parallel code sweeps of the
> relay core, auth/security, agent surface, client apps, and git/mesh/infrastructure.
> Purpose: a durable reference for building a derived project on top of Buzz.
> Where this doc and the code disagree, trust the code — file references are included
> so claims can be re-verified.

---

## 1. What Buzz Is

Buzz is a self-hostable team workspace (by Block, Inc., Apache 2.0) where humans and
AI agents are equal members. The core idea:

- **Every action is a signed Nostr event** (NIP-01 wire format) identified by a `kind`
  integer — chat messages, reactions, workflow steps, git pushes, huddle joins,
  canvas edits, agent telemetry. One event log, one identity model (secp256k1
  keypair + Schnorr signatures), one audit trail.
- **The relay is the single source of truth.** No P2P, no gossip. Clients connect to
  one relay over WebSocket (plus a narrow HTTP surface); the relay authenticates,
  verifies, persists, fans out, indexes, and automates.
- **A community = the workspace selected by the request host.** One URL, one tenant.
  Multi-tenant deployments share Postgres/Redis/S3 but the community boundary is
  enforced structurally at every layer.
- **New feature = new kind number.** Existing clients ignore unknown kinds; nothing
  breaks. All 80+ kinds are defined in `crates/buzz-core/src/kind.rs`.

## 2. Repo Layout

- `crates/` — 26 Rust crates (workspace). Key groups:
  - Relay + core: `buzz-relay` (the server), `buzz-core` (zero-I/O types/verification/
    kind registry), `buzz-db` (Postgres), `buzz-auth`, `buzz-pubsub` (Redis),
    `buzz-search` (Postgres FTS), `buzz-audit`, `buzz-media` (Blossom/S3),
    `buzz-relay-mesh` (inter-pod QUIC mesh), `buzz-push-gateway` (blind APNs).
  - Agent surface: `buzz-acp` (harness), `buzz-agent` (minimal ACP agent),
    `buzz-dev-mcp` (shell/file MCP tools), `buzz-cli` (agent-first CLI `buzz`),
    `buzz-workflow` (YAML automation), `buzz-persona`, `sprig` (all-in-one multicall
    binary).
  - Interop: `git-credential-nostr`, `git-sign-nostr`, `buzz-pair-relay` +
    `buzz-pairing-cli` (NIP-AB device pairing), `buzz-ws-client`, `buzz-sdk`.
  - Tooling: `buzz-admin` (operator CLI), `buzz-test-client` (E2E),
    `buzz-conformance` (TLA+ trace replay checker).
- `desktop/` Tauri 2 + React 19; `mobile/` Flutter; `web/` browser repo-browser SPA;
  `admin-web/` read-only admin dashboard.
- `migrations/` SQL (auto-applied on startup), `deploy/` (compose + Helm charts),
  `docs/spec/` TLA+ specs.
- Related repos (internal Block build/deploy pipelines): `squareup/sprout-releases`,
  `sprout-oss`, `block-coder-tf-stacks`, `sprout-backend-blox`.

## 3. Relay Core

### 3.1 Multi-tenancy ("row-zero binding")

- `crates/buzz-relay/src/tenant.rs`: the request `Host` header is resolved to a
  `CommunityId` **before** the WebSocket upgrade or any handler runs. Unknown/empty
  hosts fail closed (generic 404, no default tenant, no host-probing oracle).
- Community ID is then structural everywhere: Postgres PKs/indexes lead with
  `community_id`; Redis keys are `buzz:{community}:...`; moka caches key on it;
  audit hashes include it; fan-out re-checks it per recipient
  (`filter_fanout_by_access`, `handlers/event.rs:115`).
- Formally specified: `docs/spec/MultiTenantRelay.tla` (non-interference invariants,
  kept non-vacuous by mutations M1–M8). `buzz-conformance` re-implements the spec's
  transition relation independently and replays JSONL traces emitted from the live
  ingest seam (`buzz-relay/src/conformance/`); an `EmitGuard` turns "seam exited
  without emitting a trace" into a coverage failure.

### 3.2 Connection lifecycle (`connection.rs`)

1. Global connection semaphore (`try_acquire_owned`; full ⇒ drop).
2. Relay sends NIP-42 `["AUTH", challenge]`; client must sign kind:22242 within
   **5 s** (`AUTH_TIMEOUT`) or is disconnected.
3. Auth gate ladder (`handlers/auth.rs`, all fail-closed): moderation ban (with
   NIP-OA owner→agent ban cascade), optional pubkey allowlist, relay membership
   (NIP-43), NIP-OA owner backfill.
4. Three tasks per connection: recv loop (inline), send loop (batched, control
   frames prioritized; drains control before Close so ban reasons reach clients),
   heartbeat (30 s ping, 3 misses ⇒ disconnect). Slow clients: `try_send` + grace
   counter (default 3) ⇒ disconnect.
5. EVENT/REQ/COUNT handlers each take a `handler_semaphore` permit (cap 1024) and
   run spawned; AUTH/CLOSE run inline. A Redis fixed-window rate limiter gates
   admission per pubkey (5 s burst window; agent vs human budgets).

### 3.3 Ingest pipeline (`handlers/ingest.rs::ingest_event_inner`)

Ordered, load-bearing: reject relay-only/AUTH kinds → Schnorr verify on
`spawn_blocking` → timestamp drift ±900 s → content ≤256 KB → pubkey==auth identity
(exception: kind 1059 gift wraps use ephemeral pubkeys by design) → per-kind scope →
command kinds branch to `command_executor` (never stored as plain events) → ban
write-block re-check → channel derivation (`h` tag; reactions/deletions derive from
target) → membership check → per-kind validators → persistence dispatch:

- Replaceable (NIP-16) / parameterized replaceable (NIP-33): LWW under a Postgres
  advisory lock; same-second ties resolve to **lowest event id** (deterministic
  across relays); old row soft-deleted (`buzz-db/src/lib.rs:3306`).
- Reactions: atomic upsert with thread metadata.
- Replies: `reply_count` (parent) and `descendant_count` (root) bumped in the same
  transaction as the insert (`buzz-db/src/event.rs:1004`); relay then emits a
  relay-signed kind:39005 thread summary for live badges.

**`OK` means durably stored, not delivered** — Redis publish, fan-out, search, audit
enqueue, workflow triggers are spawned fire-and-forget after the insert.

Ephemeral kinds (20000–29999) bypass storage entirely: verify → (presence 20001 ⇒
Redis SET EX 90) → membership if channel-scoped → Redis publish + local fan-out.
Typing indicators (20002) are just channel-scoped ephemerals.

### 3.4 Subscriptions and fan-out (`subscription.rs`, `handlers/req.rs`)

- Registry holds **six community-scoped DashMap indexes**: (channel,kind), channel
  wildcard, global kind, global (kind,#p), global wildcard, plus the per-conn map.
  (ARCHITECTURE.md's "three-tier with linear global scan" is stale.)
- Symmetric scoping invariant: channel events never reach global subs; global
  events never reach channel subs.
- REQ flow: scope check → accessible-channels resolution (10 s cache with
  stale-negative repair against DB) → **p-gate** → optional NIP-50 search branch →
  register + `retain_topic` (Redis) → historical queries per filter
  (bounded-concurrent, 500/filter cap) with per-event re-checks
  (`filters_match`, channel access, `reader_authorized_for_event`, author-only) →
  `EOSE`. Live events then arrive via fan-out.
- **The p-gate** (`req.rs:1042`): a kindless global filter *could* match sensitive
  kinds (`P_GATED_KINDS`: gift wraps 1059, member notifications, DM-visibility
  30622, turn metrics 44200, observer frames), so it's rejected unless every `#p`
  value equals the authed pubkey. On the HTTP bridge this is the famous
  **403 for queries without `kinds`**. Defense in depth: those kinds also have a
  NULL generated `search_tsv`, so FTS physically can't find them.
- NIP-01 edge cases honored: `kinds: []` matches nothing; absent `kinds` = wildcard.
- Result-level gates: kinds 30622/44200 require reader == `#p` on every delivery
  surface, even `ids:[...]` lookups (`buzz-core/src/filter.rs:23`).

### 3.5 Cross-pod operation

- Redis pub/sub topics `buzz:{community}:channel:{uuid}` / `:global`; each pod
  subscribes only to topics with live local interest (refcounted retain/release,
  500 ms unsubscribe debounce). Local-echo dedup via a moka cache keyed
  `(community, event_id)`.
- Additional Redis control planes: cache invalidation, live ban disconnects
  (conn-control), NIP-98 replay guard, rate limiter.
- Presence: `SET buzz:{community}:presence:{pubkey} EX 90` (3× heartbeat, no flap).

## 4. Auth & Security Model

- **NIP-42** (WS): 32-byte hex challenge, kind:22242 response, ±60 s window,
  relay-URL normalization (localhost≡127.0.0.1 here — but NOT in NIP-98).
- **NIP-98** (HTTP): kind:27235 event base64 in `Authorization: Nostr ...`; URL +
  method + optional body-SHA binding; loopback hosts deliberately NOT aliased (the
  `u`-tag host is the tenant binding). Replay guard = Redis `SET NX EX` keyed by
  event id, community-prefixed, verify-then-mark, fail closed.
- **NIP-OA (owner attestation)** (`buzz-sdk/src/nip_oa.rs`): an `auth` tag inside
  the signed auth event proves an owner key delegated an agent key
  (`sig = Schnorr(SHA256("nostr:agent-auth:" || agent_pk || ":" || conditions))`).
  Strict condition grammar; self-attestation rejected; >1 auth tag ⇒ treated as
  none. Drives membership delegation, ban cascade, observer-frame auth, drafts.
- **Scopes**: NIP-42 grants all 16 scopes; real access control is NIP-29 channel
  membership (roles owner/admin/member/bot; transactional TOCTOU-safe mutations;
  open channels force self-join as Member; private channels require elevated
  inviter). Live fan-out re-checks membership per recipient at send time.
- **Audit** (`buzz-audit`): per-community SHA-256 hash chain; community_id inside
  the hash (a row moved across communities fails verification); per-community
  advisory lock (panic-safe); async lossy-by-design enqueue (bounded 1000); DMs
  audited without recording the human actor.
- **DMs**: NIP-17 gift wraps (1059) are globally stored, NIP-44-encrypted,
  `#p`-addressed; the Buzz "conversation" layer is DM channels keyed by a
  participant hash, opened via command kinds 41010–41012, with a relay-signed
  per-viewer kind:30622 visibility snapshot (result-gated).
- Rate limiting IS implemented (Redis fixed-window, `buzz-pubsub/src/rate_limiter.rs`)
  despite stale docs claiming otherwise.

## 5. Storage

- **Postgres** (`schema/schema.sql`): `events` PK `(community_id, created_at, id)`,
  monthly range-partitioned; JSONB tags; soft deletes; generated
  `search_tsv` tsvector column (FTS index IS the row — no sidecar indexer; NULL for
  privacy-sensitive kinds). Channels, members (soft-delete), thread_metadata,
  reactions, relay_members, moderation, workflow tables, push-lease tables. 24
  migrations, sqlx runtime queries (no compile-time cache).
- **Redis**: fan-out, presence, typing (ZADD + 5 s window), rate limits, replay
  guards, cache invalidation, conn control, mesh ready-registry, fenced leases.
- **S3/MinIO** (bucket `buzz-media` by default) holds BOTH:
  - **Media** (Blossom): raw blobs content-addressed `{sha256}.{ext}` (shared CAS),
    plus per-community metadata sidecar `_meta/{community}/{sha256}.json` which is
    the tenant read gate. Upload `PUT /media/upload` (kind:24242 Blossom auth +
    relay membership), download `GET /media/{sha256}.{ext}` via the relay only.
    Defaults: files 100 MB, images 50 MB, video 500 MB, GIF 10 MB;
    `BUZZ_REQUIRE_MEDIA_GET_AUTH` defaults to false (unauthenticated reads of
    unguessable hashes — flip on to gate reads). No listing/browse endpoint —
    file discovery is only through messages carrying `imeta` tags. (A "shared
    drive" UX does not exist; closest substitutes: a files channel, canvases
    kind 40100, or a hosted git repo.)
  - **Git repos**: immutable content-addressed packs + one manifest pointer swapped
    by S3 CAS (`If-Match` etag). Every request hydrates a tempdir from S3, runs
    real `git --stateless-rpc` (hardened env), discards it. Push success response
    is constructed only after CAS commit (proven in
    `docs/spec/GitOnObjectStore.tla`); a fail-closed startup probe races the
    backend to verify CAS semantics. Packs are never deleted (concurrent-reader
    safety). Relay-signed kind:30618 ref-state event emitted after push (derived
    notification, never the commit point).

## 6. Git Surface

- Smart HTTP at `/git/{owner}/{repo}` with NIP-98 auth on every route; method
  binding and replay dedup deliberately disabled for git (credential-protocol
  constraint — documented tradeoff; security = ±60 s window + URL lock + HTTPS +
  policy hook).
- Fail-closed pre-receive policy hook → HMAC'd POST to localhost-only policy
  endpoint → `buzz-core/src/git_perms.rs` maps channel roles to repo permissions;
  `buzz-protect` rules from kind:30617 tags (no-force-push, no-delete,
  require-patch, per-ref push:role — explicit rules can only tighten defaults).
- `git-credential-nostr`: credential helper signing NIP-98 tokens (+ NIP-OA tag via
  `x-auth-tag`). `git-sign-nostr`: BIP-340 commit/tag signing shim (NIP-GS) with
  serious secret hygiene.
- NIP-34 kinds: 30617 announcement, 30618 state, 1617 patch, 1618 PR, 1621 issue,
  1630–1633 statuses.

## 7. The Agent Surface

**Central fact: the harness never posts the agent's replies.** The agent replies by
running `buzz messages send` itself, via the shell tool, using credentials the
harness injected. The harness only routes events in, supervises processes, posts
👀/💬 reactions, and streams encrypted telemetry.

Chain: Desktop → spawns `buzz-acp` → spawns agent binary (goose / codex-acp /
claude-agent-acp / buzz-agent) over ACP (stdio NDJSON JSON-RPC) → agent's MCP server
is `buzz-dev-mcp` (shell, read_file, str_replace, view_image, todo) with env
`BUZZ_PRIVATE_KEY` / `BUZZ_RELAY_URL` / `BUZZ_AUTH_TAG`.

- `buzz-acp`: NIP-42 WS connection; subscribes to mentions (default) or rule-filtered
  events; per-channel queueing with at-most-one-in-flight; agent pool (1–32) with
  session affinity; mid-turn **steer** (falls back to cancel+merge); owner control
  commands (`!shutdown`/`!cancel`/`!rotate`); prompt assembled from
  `base_prompt.md` + system + team instructions + NIP-AE core memory + channel
  context; publishes kind:44200 turn metrics (encrypted to owner) and kind:24200
  observer frames (`BUZZ_ACP_RELAY_OBSERVER=true`). Codex gets
  `sandbox_workspace_write.network_access=true` forced so Seatbelt doesn't block
  the CLI.
- `buzz-agent`: minimal ACP agent; providers are a plain enum match (Anthropic /
  OpenAI / Databricks); non-streaming; tool-calls-as-output loop; MCP children get
  a whitelisted env (API keys never leak); built-in `load_skill`; context handoff
  by self-summarization.
- `buzz-dev-mcp` shim: creates a tempdir of **multicall symlinks** (`buzz`, `rg`,
  `tree`, git helpers → its own binary) prepended to PATH; `sprig` is the same
  trick one level up (argv[0] dispatch to acp/agent/dev-mcp).
- `buzz-cli`: REST (NIP-98) for almost everything via `POST /events` / `/query`;
  WS only for ephemeral kinds (bridge rejects them). JSON out; exit codes 0/1/2/3/4
  (+5 write conflict). **Owner-reviewed draft flow**: `buzz agents draft-create`
  sends a kind:24200 encrypted draft that opens a prefilled form on the owner's
  desktop — nothing changes until the human saves.
- `buzz-workflow`: runs inside the relay (`workflow_engine.on_event` spawned post-
  store). Triggers: message_posted / reaction_added / diff_posted / schedule
  (60 s tick) / webhook (`/hooks/{id}`). Actions: send_message, send_dm*,
  set_channel_topic*, add_reaction, call_webhook (SSRF-guarded), request_approval*,
  delay. (* = stubbed/partial, see §10.) evalexpr conditions with registered string
  helpers, spawn_blocking + timeout. Relay-signed outputs tagged `buzz:workflow`
  to prevent trigger loops.
- `buzz-persona`: `.persona.md` (YAML frontmatter + markdown system prompt) and
  pack directories; resolves to ACP config incl. per-runtime env projection
  (model → GOOSE_* / BUZZ_AGENT_*).
- Desktop (`src-tauri/managed_agents/`): spawns/supervises harness processes,
  layered env (reserved keys like BUZZ_PRIVATE_KEY are stripped from user env),
  readiness/"setup mode" (unconfigured agents run a nudge listener instead of an
  LLM), Unix process groups / Windows Job Objects for tree-kill, orphan sweeps.

## 8. Clients

| Client | Keys | Signing | Transport |
|---|---|---|---|
| Desktop (Tauri 2 + React 19) | OS keyring (`buzz-desktop`/`identity`), 0600 file fallback | Rust | **Reads: native Rust WS** (Tauri plugin, not WebView). **Writes/history: HTTP bridge** with NIP-98 |
| Mobile (Flutter/Riverpod) | FlutterSecureStorage, per community | Dart | Own WS session (Dart port of desktop's session) |
| Web (React SPA) | NIP-07 extension or ephemeral key | Browser | Plain WS + NIP-98; isomorphic-git in browser (IndexedDB clones) |
| Admin-web | none | n/a | GET-only `/api/admin/v1/*`; auth = host/Origin gating only (network isolation!) |

Desktop specifics: React Query is the store (no Redux); events batched every 16 ms
off the socket; history fetches request content kinds only, aux kinds (reactions/
edits/deletes) backfilled by `#e`; community switching = full React remount keyed on
community id + explicit reset of every module singleton
(`resetCommunityState()` in `features/communities/useCommunityInit.ts` — **any new
community-scoped module-level cache must register a reset there**). Desktop also
runs the huddle audio stack (Opus WS + jitter buffer + STT/TTS so agents can join
voice) and a local authenticated media proxy.

Mobile gets its identity by **QR pairing from desktop** (NIP-AB: ephemeral ECDH +
6-digit SAS + NIP-44 transfer through `buzz-pair-relay`, an amnesiac sidecar relay
that only ever sees ciphertext). Mobile push today = local badges from live events;
the server-side push gateway exists but device-token registration isn't in the
Flutter app yet.

**Push gateway** (`buzz-push-gateway`): blind by construction — one fixed compiled-in
"reconnect to your relay" APNs payload; DeliveryRequest has no payload field;
capabilities are opaque AES-256-GCM grants (no APNs token inside); devices enroll
via Apple App Attest; leases are NIP-44-encrypted kind:30350 events with strictly
validated narrow filters (push-eligible kinds only: 7, 9, 1059, 40007, 46010;
gift wraps only match their own `#p` to prevent wake-timing leaks).

## 9. Scaling & Ops

- Stateless relay pods: Postgres + Redis + S3 hold all state. Horizontal scale via
  Redis fan-out + `buzz-relay-mesh` (iroh/QUIC, scuttlebutt membership, phi-accrual
  failure detection). **Fencing law: "mesh membership is a hint; the Redis fenced
  lease generation is the arbiter"** — every cross-pod datagram carries a
  generation; stale ⇒ dropped. Used for cross-pod huddles, session tunnels.
  Kill switch `BUZZ_MESH=off`.
- Separate concept: **mesh-llm** (external llama.cpp-style compute SDK) — desktop
  feature (`--features mesh-llm`) letting trusted members share local inference;
  admission = allowlist from the cryptographic member roster (kind:13534) with
  owner attestation required; roster shrink never de-admits on transient failures.
- Deploy: Dockerfile (multi-stage, ships buzz-relay + buzz-admin + buzz-pair-relay
  + built web/admin SPAs), docker-compose (postgres:17, redis:7, minio, adminer,
  keycloak, prometheus), Helm chart (`deploy/charts/buzz/`) with HPA/PDB/
  ServiceMonitor + separate push-gateway chart.
- `buzz-admin`: add/remove/list members (publishes kind:13534 roster via Redis,
  `created_at = max(now, newest+1)` to defeat same-second domination), generate
  keys, migrate, feedback, channel-discovery backfill. Owner role is config-only
  (`RELAY_OWNER_PUBKEY`), never grantable via CLI.

## 10. Known Gaps & Stale Docs (verified July 2026)

1. ARCHITECTURE.md says rate limiting is unimplemented — **stale**; Redis limiter
   is wired and enforced (WS admission + HTTP bridge).
2. ARCHITECTURE.md's "three-tier fan-out with linear global scan" — **stale**;
   now six community-scoped indexes, sub-linear global fan-out.
3. Workflow approval gates suspend but don't resume (runs marked Failed; WF-08).
4. Workflow `send_dm` / `set_channel_topic` actions return NotImplemented (WF-07).
5. Huddle recording / per-track publishing: kinds reserved, no producer.
6. Mobile push: gateway complete server-side; Flutter token registration absent.
7. No media listing/browse endpoint (no "shared drive" UX).
8. sqlx runtime queries — no compile-time SQL validation.

## 11. Building On Top (practical notes)

- **License**: Apache 2.0 — commercial use, modification, and distribution are
  permitted; keep the LICENSE and copyright/NOTICE attributions, and state
  significant changes. Contributions upstream go through a CLA (see
  CONTRIBUTING.md / GOVERNANCE.md). (Not legal advice.)
- **Extension pattern the codebase expects**: model new operations as new event
  kinds — add the constant in `buzz-core/src/kind.rs`, handle in `buzz-relay`
  (scope in `required_scope_for_kind`, validators in ingest), mirror the kind in
  `desktop/src/shared/constants/kinds.ts` and
  `mobile/lib/shared/relay/nostr_models.dart`. Avoid new HTTP endpoints — the
  generic bridge (`POST /events` / `/query` / `/count`) plus a kind usually
  suffices and gets fan-out, scoping, and auth for free.
- **Agent-facing features** go in `buzz-cli` first (subcommand + `client.rs` call).
- **Channel scoping** is `h` tags (NIP-29), not `e` tags. Relay queries must
  specify `kinds` or they hit the p-gate 403.
- **Quality gates**: `just ci` (fmt+clippy+lint+unit+builds) before any PR;
  `just test` (integration; needs Postgres+Redis) if touching relay/db/auth.
  No `unsafe`; no new `unwrap()`/`expect()` in production paths; doc comments on
  new public API. Desktop text sizes must use rem tokens (zoom support).
- **Danger zones for forks**:
  - The ingest pipeline ordering and the fail-closed gates are load-bearing;
    reorder with extreme care.
  - Anything touching tenancy must keep community_id structural (PKs, Redis keys,
    cache keys, hashes) — the conformance suite will catch violations if you keep
    emitting traces.
  - Desktop: new community-scoped module singletons must register in
    `resetCommunityState()` or state leaks across community switches.
  - The git subsystem's correctness rests on S3 CAS; don't swap in a backend
    without the startup probe passing.
- **Key entry points to read first**: `buzz-relay/src/main.rs` (boot),
  `router.rs` (surface), `connection.rs` (WS lifecycle),
  `handlers/ingest.rs` (the pipeline), `subscription.rs` (fan-out),
  `buzz-core/src/kind.rs` (the vocabulary), `buzz-acp/src/lib.rs` (agent loop),
  `desktop/src/shared/api/relayClientSession.ts` (client session).
