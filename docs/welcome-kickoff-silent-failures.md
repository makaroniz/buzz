# Welcome Kickoff — Failure Paths

Status: **partly open.** The *perception* gap is handled (see below); the silent
paths themselves are not. The [intro loop](#the-intro-loop-a-loud-failure) is
mitigated as of 2026-07-18 by three bullets in the base prompt's Callback
Mentions section, and **confirmed live against Codex** — the chain terminated on
its own at four replies. Prompt is the only lever available: a structural cap was
considered and rejected (see that section). The
[double closer](#the-closer-was-racing-a-message-it-could-not-beat) that the same
run exposed is fixed as of 2026-07-18.
Context: the Welcome-channel kickoff choreography
(`desktop/src/features/onboarding/welcomeKickoff.ts`) where Fizz posts an
opener and teammates introduce themselves in-thread.

Two failure shapes live here: the **silent** ones (nobody speaks) and the
**loud** one (nobody stops). They have opposite symptoms and the same root
cause — the kickoff assumes agent behavior it never constrains.

## The silent-path problem

Every fallback message in the kickoff assumes Fizz — the lead agent and
sender — is alive and able to post. When Fizz itself fails, or an early step
throws, **nothing is ever posted** and the user stares at an empty Welcome
channel with no explanation.

The client-side kickoff stage (the starter-team characters standing on the
Welcome composer banner) covers the *perception* gap, and that part has landed:
after `WELCOME_KICKOFF_STAGE_TIMEOUT_MS` (90s) with no message, the characters
play their exit and the banner drops back to its normal mention hint. A failed
kickoff degrades to an ordinary, usable empty channel rather than claiming a
team is still being set up.

What it does **not** do is explain anything — the silent paths below still need
real handling. Two things worth knowing before picking this up:

- The stage is driven purely by "is the timeline empty" plus that timer
  (`useWelcomeKickoffStage.ts`). It never reads the real kickoff state, so it
  cannot distinguish "Fizz crashed" from "the relay is slow" — the timeout is a
  perception backstop, not a diagnosis. Surfacing a cause means plumbing one out
  of `useWelcomeKickoff` (step 1 of the Sketch below).
- The empty channel it degrades to invites the user to `@`-mention Fizz — who,
  in exactly these failure cases, is the thing that isn't working. So the quiet
  timeout is honest but still a dead end.

## Message inventory (what the user CAN receive today)

All hard-coded client-side; only teammate intro replies are LLM-generated —
which is exactly where the [intro loop](#the-intro-loop-a-loud-failure) lives.
Everything the client authors is bounded; the one unbounded part is the part
that runs away.

| # | Message | Trigger | Sender |
|---|---------|---------|--------|
| 1 | Provider fallback ("connect to an AI provider in Settings…") | Readiness check fails before kickoff | Fizz (marker: `provider-required.v1`) |
| 2 | Happy-path opener (mentions teammates, asks them to introduce themselves) | Team online | Fizz (marker: `opener.v1`) |
| 3 | Degraded opener ("I'm here with Honey and Bumble…") | Fizz online, zero teammates online within 60s | Fizz (opener + closer markers, self-contained) |
| 4 | Closer variants (failed / slow teammate wording only — a clean kickoff posts nothing) | 3s beat after a teammate is seen crashed, or 60s intro timeout | Fizz (marker: `closer.v1`) |
| 4b | The actual close on a clean kickoff | The intros `@mention` Fizz, waking it | Fizz, **LLM-generated** (no marker) |
| 5 | Setup-mode nudge ("here's what you still need to configure") | Agent process spawns but its requirements check fails (e.g. missing API key) | The agent process itself (backend, buzz-acp setup-listener mode) |

## Silent paths (what the user CANNOT be told today)

1. **Fizz fails to start.** `startManagedAgent` for the lead rejects (harness
   binary missing, spawn error). The kickoff effect logs
   `Failed to start Welcome agent…` and returns — by design, the opener is
   only sent by Fizz, so nobody speaks.
2. **Any step throws.** The entire kickoff runs in one `try/catch` that logs
   `Failed to start the Welcome team kickoff.` and gives up. Causes seen in
   practice:
   - relay unreachable / websocket down
   - `ensureWelcomeTeam` failure (team record creation)
   - the send itself rejected — e.g. relay rate-limiting
     ("rate-limited: quota exceeded", observed 2026-07-17 with an agent
     publishing in a tight retry loop)
3. **Closer-path failures.** The closer send failing is also caught-and-logged
   only; the thread ends without the CTA. Lower stakes than 1–2 (an opener and
   intros already happened) but still a dangling state.

Navigation away mid-kickoff also cancels silently, but that is intentional
(the kickoff resumes on next visit) — not a failure.

## The intro loop (a loud failure)

Observed 2026-07-18 with **Codex** as the provider: the intro thread never
terminates. Fizz, Honey, and Bumble keep replying to each other — 21+ replies,
each one an agent announcing it will now stop replying ("parked", "acknowledged",
"I won't reply again unless there's a task for me"), each announcement waking the
others. **Not reproduced with Claude Code.**

### Why it loops

The wake trigger is `p`-tag gated: an agent fires when a message mentions it
(`require_mention`, default on — `crates/buzz-acp/src/filter.rs:390`). A reply
adds `p` tags **only** for names explicitly `@`-mentioned in its body
(`buzz-sdk/src/builders.rs:218`; threading itself contributes `e` tags only). So
mentioning a teammate — for any reason, including to say goodbye — is
indistinguishable from summoning them.

Two unconditional rules in the shared base prompt then close the cycle:

| `crates/buzz-acp/src/base_prompt.md` | Rule |
|---|---|
| `:44` | "When you finish delegated work, you **MUST** `@mention` the delegator in your completion message." |
| `:61` | "**Every turn that processes a user message MUST publish a reply.** … A turn that ends without a published message is a silent failure." |

Composed, they have no exit:

```
Fizz's opener @-mentions Honey + Bumble        (kickoff, by design)
  → Honey finishes her intro ("delegated work")
  → :44 — MUST @mention the delegator          → Fizz wakes
  → Fizz processed a message
  → :61 — MUST publish a reply                 → Fizz replies
  → :44 — acknowledgment @-mentions the two    → Honey + Bumble wake
  → … (forever)
```

**The sign-off is the trigger.** "I'll stay quiet until you have a task" is a
mention, and a mention is a wake. Politeness is the fuel.

Nothing structural brakes this:

- `ignore_self` (`lib.rs:1864`) filters an agent's **own** pubkey only — it does
  not touch A↔B mutual mentions.
- The author gate (`RespondTo::OwnerOnly`, the default) admits **siblings** —
  other agents launched by the same owner (`is_owner_or_sibling`,
  `lib.rs:166–190`). Agent-to-agent traffic is allowed by design.
- `max_turns_per_session` defaults to `0` = unlimited (`config.rs:372`).
- There is no per-thread agent-to-agent turn cap and no human-in-the-loop check.

So the only brake is model discretion — which is precisely the variable that
differs between providers.

### Why Codex and not Claude Code

**No code path explains it.** Prompt content is identical for both (both are
protocol ≥ 2, both receive the same `base_prompt.md` + persona `[System]` via
`pool.rs:1072`). The only per-provider prompt divergence is Goose (custom
request) and legacy protocol `< 2` (inlined `[System]`) — neither is
Codex-vs-Claude. Codex's only special-casing is network-sandbox env injection
(`config.rs:640–668`), which has no prompt effect.

The difference is behavioral: Codex reads `MUST` as a hard constraint and obeys
it literally every turn; Claude Code treats it as defeasible and exercises
discretion about when silence is correct. **Which means the current design only
ever worked by luck** — it depends on a model choosing to violate an explicit
instruction. Any provider that follows instructions faithfully will loop. Codex
is not misbehaving here; it is doing what the prompt says.

Corollary for the fix: **prompting alone cannot be the whole answer.** Every
message in the observed thread is an agent *promising to stop*. They understood
the instruction and complied verbally while structurally continuing to loop. A
prompt that says "don't loop" is one more sentence for a literal-minded model to
acknowledge — in a reply, with a mention.

### Where the prompt fix belongs

`crates/buzz-acp/src/base_prompt.md` — **not** the personas, **not** the team
instructions. Reasoning:

| Layer | File | Verdict |
|---|---|---|
| Base prompt (all agents) | `crates/buzz-acp/src/base_prompt.md` | ✅ **Right home.** The two rules that *cause* the loop live here, unconditional. The cycle is a general property of any mutual-mention agent set — not a Welcome-team quirk. |
| Team instructions | `desktop/src-tauri/src/managed_agents/teams.rs:59` (`instructions: None` — slot exists, unused) | ⚠️ Scoped to the Welcome Team only. Same loop reappears for any other team or ad-hoc pair. |
| Per-persona | `desktop/src-tauri/src/managed_agents/personas.rs:22–26` (Fizz/Honey/Bumble) | ❌ Three copies to drift, and every new persona re-introduces the bug. |

Fixing it above the base prompt would leave `:44` and `:61` still saying "always
reply, always mention the delegator" and layer a contradiction on top. The rules
need a **termination clause at the source**: bound the "MUST reply" to turns that
advance work, and carve out acknowledgment-only replies from the "MUST mention
the delegator" rule (an ack that needs no action should either not be sent or not
mention). Both edits belong in the same two bullets that created the cycle.

### Constraints for the intro-loop fix

- **Don't break callback mentions.** `:44` exists because a missing callback is
  "the #1 cause of stalled collaboration" — the reason the rule is unconditional.
  Loosening it trades a loud failure for a silent one, which is the failure class
  this doc already tracks. The carve-out must be narrow: acknowledgments that
  close a loop, not completions that hand work back.
- **Don't break `:61` either.** It exists to prevent invisible turns. "Reply
  unless you have nothing to add" invites a model to skip a real answer.
- Prompt change is **necessary but not sufficient** — pair it with a structural
  brake so a literal provider cannot loop even if it ignores the prose.

### The fix (landed 2026-07-18)

**Give the acknowledgment somewhere else to go.** The loop is not caused by
agents wanting to talk too much; it is caused by the only available way to say
"seen" being a mention, and a mention being a wake. So bound the MUST that forces
the mention, and hand the impulse a zero-cost outlet.

Three bullets, all in **Callback Mentions** (`base_prompt.md:42`). Nothing else
changed — no persona, no team instructions, no kickoff copy, no harness code:

1. **Bound the callback.** It hands work *back* — a completion, a blocker, or a
   question — is sent **once**, and a delegator can waive it. The unconditional
   MUST itself is **kept**: stalled collaboration is the worse failure (see
   Constraints).
2. **Forbid the ack-mention, and say why.** Never `@mention` anyone *just* to
   acknowledge, agree, confirm, or sign off — *"a mention is a summons… a tag
   costs a whole turn, it is not punctuation… 'I won't reply again' does not end
   a conversation, it restarts it."* A literal model needs the mechanism, not
   just the rule.
3. **Point at reactions as the alternative.** *"To acknowledge, use a reaction
   instead — `buzz reactions add`."* This is the load-bearing addition: it
   redirects rather than suppresses. Telling a model "don't" leaves it with an
   unresolved impulse; giving it a `p`-tag-free way to signal "seen and agreed"
   resolves it, **and keeps the human-visible social signal** so the fix doesn't
   read as stalling. `buzz reactions add` already existed and is already in the
   command table (`:13`) — the base prompt just never connected it to acking.

### Why `:61` was left alone

An earlier draft also rescoped `:61` ("Every turn that processes a user message
MUST publish a reply") to humans only, plus an explicit "end the turn silently
when the exchange is over". **Reverted.** Two reasons:

- **It isn't load-bearing.** A loop needs A's message to *wake* B, and wake is
  `p`-tag gated. `:61` forces a *reply*, not a *tag* — an untagged reply ends the
  chain after one message. `:61` governs **noise**, not the loop.
- **It was the riskiest line in the diff.** Any weakening of "always reply" trades
  toward the silent failures this same doc tracks. Not worth it for noise.

Corroborating data point (2026-07-18): another Buzz agent (`Eva`) runs the same
base prompt **including** `:61`, plus a per-agent core memory that says almost
exactly bullets 2–3 above ("Don't @-tag another agent to merely acknowledge…
ack with a reaction instead"; "a tag = forced wake-up = a whole invocation").
It does not loop. So the mention rule alone is sufficient, and `:61` can stay
as-is.

That memory also **confirms the diagnosis from the other direction**: its
"complementary rule" (only mention when you need attention, never in narrative)
was *already in our base prompt at `:40`* the whole time — and the agents still
looped, because a soft "only… when" loses to a hard MUST two lines below it. The
per-agent memory works because it scopes the mandatory tag narrowly ("completion
callbacks are the one mandatory tag") — the same move as bullet 1. Memory fixed
one agent; the base prompt is where it holds for every agent, including a fresh
Fizz with no memory.

**Known residual tension:** `:61` still says a turn MUST publish a reply, which a
literal model may read as outranking "ack with a reaction". Worst case is an
extra untagged message — noise, not a loop, since it wakes nobody. Accepted
deliberately; revisit only if observed.

**Deliberately NOT done — telling the teammates not to mention each other in the
kickoff opener.** Tried and reverted, for two reasons:

- **Product.** The opener is the first thing a new user ever reads. A greeting
  that visibly negotiates anti-loop rules with its own agents ("don't @mention me
  or each other") advertises that the product needs intense prompting to behave
  normally. The fix belongs somewhere the user never sees. The opener now carries
  **no agent-steering instructions at all** — "Don't start any work yet" went too
  (nothing has been asked for yet, so it guarded against nothing concrete, and
  "in a sentence or two" already bounds the intros). Pinned by a test, so
  re-adding one is a deliberate decision rather than a reflex.
- **Isolation.** Shipping it alongside the base-prompt edit confounds the test:
  the opener waiver alone would suppress the loop in the Welcome channel, so a
  clean run would prove nothing about whether the base prompt actually works —
  and the base prompt is the part that has to hold for *every* agent in *every*
  channel, not just this one thread. Keeping the opener ordinary makes the
  Welcome kickoff an honest test of the general fix.

The `:44` delegator waiver stays in the base prompt as a general capability (any
delegator, human or agent, can say "no need to report back") — it is simply not
exercised by the kickoff.

**Deliberately NOT done — a structural turn cap.** Considered and rejected: a
cap on consecutive agent-authored turns cannot distinguish a loop from a
long-running task where agents legitimately trade many messages, so it would
truncate real work. There is no cheap structural signal that separates the two —
the difference between "parked, standing by" and a useful update is semantic.
That leaves prompting as the only lever, with the honest caveat below.

**The closer needs no change** — its happy path is the bare CTA and its degraded
paths use plain names, not `@`, so it emits no `p` tags and cannot wake anyone.

### Residual risk

Prompting is the only mitigation, and prompting is what failed here: every
message in the observed thread was an agent promising to stop. The bet is that
the old prompt left a compliant model **no way to say "seen" except a mention**,
so it complied verbally while looping structurally — and that bounding the MUST
plus offering reactions as the outlet removes the loop. That is a real bet, not a
guarantee. It cannot be verified by unit tests; it needs a live Codex run on the
Welcome flow (see Verification below).

The `Eva` data point (above) is the strongest evidence it will hold: an agent
carrying essentially these rules in memory demonstrably does not loop. The
untested part is whether the same words work at **base-prompt salience** rather
than per-agent-memory salience — memory sits closer to the model, and `:40`
already proved that a low-salience mention rule loses to a nearby MUST.

If it recurs, the next lever is not a blunt cap but a **semantic** one — e.g. the
harness declining to wake an agent on a sibling mention whose message adds no
new work — which needs a judgment call the harness can't currently make.

### Verification status

- **Verified live against Codex on 2026-07-18, with the base prompt as the only
  variable** (the opener waiver was reverted first, precisely so this run would
  isolate it). The chain terminated on its own: intros tagged `@Fizz` (the
  legitimate `:44` callback), Fizz woke and replied once using plain narrative
  names and **no tags**, so nobody woke and the chain ended at four replies. The
  designed exit path is the one that fired.
- An earlier run went from 21+ replies to 2, but it had the opener waiver in it
  and is **confounded** — it does not count as evidence for the base prompt. It
  is the reason the opener was reverted.
- Prompt edits have no test that can fail on a literal-minded model, so the
  desktop/Rust suites say nothing about whether this holds. One live run is
  encouraging, not proof: n=1 on a sampled model.
- Unit-tested only: the solo opener mentions no agent at all (pins the
  degraded path, which cannot loop by construction).

### Outstanding questions

- Which other providers loop? Only Codex is confirmed, and **Codex-only is not a
  safe assumption** — the fix is deliberately provider-agnostic and written to
  be correct for a *perfectly obedient* model. Goose and the legacy `< 2`
  inlined-`[System]` path take a different prompt route (`pool.rs:181–191`) and
  are untested.
- If the base prompt alone does **not** hold, the opener waiver is the known
  fallback — it worked in the confounded run above. The cost is the product one:
  the first message a new user reads would carry anti-loop instructions. Prefer
  fixing the base prompt wording first.
- Do agents actually reach for `buzz reactions add`? The rule is only as good as
  the model's willingness to use a tool instead of writing prose. If it gets
  ignored, the ack-mention prohibition degrades from "redirect" to "suppress",
  which is the weaker form.
- Does "never mention just to acknowledge" bleed into pickup acknowledgments for
  real work (`base_prompt.md:66` expects a pickup ack before follow-up tools)? A
  pickup ack *precedes* work and is a legitimate mention; a terminal ack follows
  the end and is not. The wording targets "acknowledge, agree, confirm, or sign
  off", which reads terminal — but the boundary is a model judgment, and reading
  it too broadly converts this loud failure into the silent kind.

## The closer was racing a message it could not beat

Found in the same 2026-07-18 Codex run that cleared the intro loop. The user saw
Fizz say *"Honey and Bumble are taking longer than expected"* — and then both
intros landed one minute later, followed by Fizz closing again, properly. Two
CTAs, and the wrong one came first.

Neither symptom was a failure state. They were **one design conflict**:

1. **The closer raced the intros on a 15s stopwatch.** `TEAMMATE_INTRO_WAIT_MS`
   was `15_000`, plus a `CLOSER_BEAT_MS` 3s beat — about 18s for a teammate to
   have an intro *published*. That has to cover a whole agent turn: wake on the p
   tag, fetch context, run inference, shell out to `buzz messages send`. Real
   turns run tens of seconds, so the closer reliably lost and announced a delay
   that wasn't real. PR #2066 tightened this beat, which made it more likely, not
   less.
2. **The scripted closer could never be the last word.** The choreography assumed
   it was. But the intros `@mention` Fizz — that is the *mandatory* callback in
   `base_prompt.md:44`, since Fizz delegated the intros — so Fizz is **guaranteed**
   to wake and reply after them. The choreography and the base prompt were
   fighting: one scripted a final message, the other guaranteed a message after
   it.

Worth noting which one was better: Fizz's live reply ("bring us a project, bug,
question, or half-formed idea and I'll route it or start building") beat the
scripted CTA. It was contextual, and it was right about the state of the world.

### The fix (landed 2026-07-18)

**Let the guaranteed message be the close, and keep the script for problems only.**

1. `buildWelcomeKickoffCloser` returns **`null`** for a clean kickoff — the
   success state posts nothing. The failure variants (crashed teammate, slow
   teammate) are unchanged and still carry the CTA, because there the user needs
   both the bad news and a way forward.
2. The `null` is enforced inside `sendWelcomeKickoffCloser`, not at the call
   sites. There are two callers, and the delayed-teammate timer can fire *after*
   the intros land and re-classify the kickoff as clean — the choke point stops
   that stray timer from emitting a bare CTA.
3. `TEAMMATE_INTRO_WAIT_MS` `15_000` → `60_000`, matching the presence wait. This
   budget now only covers a teammate that is *alive but silent*, where patience is
   the correct answer — a **crashed** teammate is read from agent status
   (`failed`) and closes immediately, so real breakage is still reported fast.

### The latch this broke, and why it needed a second signal

The closer marker was doing quiet double duty: it was also the durable *"kickoff
already finished"* signal, in two places — retiring the opener-thread watch, and
the guard in the start effect that stops the choreography re-arming on revisit.
A marker only exists if a **message** exists (`markerExists` resolves it by
querying the relay for a tagged message), so "success posts nothing" silently
means "success never latches". Left alone that would have:

- refetched the opener subtree forever on every Welcome revisit — the exact bug
  PR #2066 fixed; and worse,
- **restarted the whole Welcome team every time the channel was opened**, because
  the start effect's guard would never trip.

So completion needs a second piece of evidence, and it has to be **relay-side** —
a local latch would re-run on another device or after a reinstall.
`welcomeKickoffAlreadyFinished` now reads: *closer marker exists* **OR** *the
opener's thread holds an intro from every teammate*. Both are durable server-side
facts. The marker still short-circuits first, so the failure paths and the solo
opener (which carries both markers on one message) are untouched and cost no
extra fetch.

The in-memory latch stays a latch for the reason the original comment gives: the
evidence lives in the subtree that the latch itself gates, so deriving it fresh
each render is self-referential.

### Residual risk

- **The CTA is now model-generated on the happy path.** If Fizz wakes and says
  something that doesn't invite the user in, the kickoff ends softer than the
  script did. The mitigation is `base_prompt.md:44` making the wake itself
  guaranteed — but *what Fizz says* is not. This is the deliberate bet of the
  change, and the thing to watch across runs.
- **If Fizz doesn't reply at all, nothing closes.** The intros still landed, so
  the channel is a usable welcome rather than an empty one — but there is no CTA
  and no scripted backstop. A watchdog (wait N seconds for the lead's callback,
  then post the scripted CTA) was considered and deliberately left out: it re-adds
  the timer race this change removed. Revisit only if a run shows the lead going
  quiet.
- Unit tests pin the contract (`buildWelcomeKickoffCloser([])` is `null`; the
  finished-check reads the intros with no marker; a marker short-circuits without
  a fetch; a missing intro is not finished). What they **cannot** pin is whether a
  real Fizz reliably produces a good close — same limitation as the intro-loop
  fix.

## Constraints for the silent-path fix

- **Fizz cannot be the messenger** for these paths: she is the thing that
  failed. Any user-visible fallback must come from the client UI itself
  (banner, intro-block state, or the kickoff stage's `timed-out` phase) — not a
  channel message impersonating an agent.
- A relay-side or system-authored message is possible in principle
  (kind-scoped system event) but heavier; the client already knows locally
  that the kickoff threw, so a local UI state is the cheap, honest option.
- Whatever surfaces must be **idempotent across revisits** — same rule as the
  opener markers: don't re-alarm the user every time they click Welcome.
- Distinguish *retryable* (relay hiccup, rate-limit) from *actionable*
  (harness missing → point at Agents/Settings). The `Requirement` machinery
  in `desktop/src-tauri/src/managed_agents/readiness.rs` already classifies
  the actionable ones.

## Sketch for the silent paths (to validate later)

1. Surface a `kickoffError` phase from `useWelcomeKickoff` when the catch
   block fires or the lead's start rejects, with a coarse cause
   (`lead-start-failed` | `relay` | `unknown`).
2. The kickoff stage's `timed-out` phase renders that cause: quiet copy + a
   pointer to Agents (for start failures) or a retry affordance (for relay
   failures). Retry = re-run the effect (the coordinator already dedupes).
   Note the phase currently exits immediately on timeout — giving it copy to
   show means holding it on screen instead, and the stage is `aria-hidden`
   decoration today, so anything it says needs to reach screen readers too.
3. Consider a bounded auto-retry (once, after a short delay) for the relay
   class before showing anything.
4. Closer-path failure: on send rejection, retry once; otherwise leave the
   thread as-is (intros already delivered the core experience).

## Related

- Rate-limiting incident: one Welcome agent produced a 42KB log of
  "rate-limited: quota exceeded" within seconds (2026-07-17, remote relay
  `onboarding.communities.buzz.xyz`).

  **Rate limiting is downstream of the intro loop, not a cause of it** — checked
  2026-07-18, and this corrects the earlier note here that assumed "an agent
  publishing in a tight retry loop":
  - That string is emitted by the **relay rejecting a publish**
    (`buzz-relay/src/connection.rs:652`), so the log is 42KB of *rejections*,
    not of client retries.
  - There is **no retry/backoff code in buzz-acp's publish path** — the only
    `fetch_with_retry` usage is for context fetches (`pool.rs:2159`). Nothing
    supports the tight-retry-loop reading.
  - Rate limiting cannot *drive* a back-and-forth: a rejected send produces no
    event, hence no `p` tag, hence no wake. A rate-limited send **breaks** the
    loop rather than feeding it.

  So the causal arrow is loop → publish volume → quota → rejections. **The intro
  loop fix does not depend on any rate-limit work**, and the two can be worked
  independently.

  Still open (hypothesis, unobserved): a provider whose *send* fails may retry it
  within its own agent loop, since `:61` frames an unpublished message as a silent
  failure — a literal model may not accept a failed send either. If the 42KB came
  from one agent's repeated `buzz messages send` attempts, that is the mechanism.
  **This one is not addressed here** (`:61` was deliberately left as-is — see
  "Why `:61` was left alone"), so it remains live: a quota rejection has no
  client-side brake at all. Worth a look at publish backoff independently.
- The two failure classes interact: silent-path work adds messages, and every
  new message is a potential wake. Any fallback that `@mentions` an agent to
  recover from a failure can itself seed a loop.
- **Open thread panes can go stale while the reply count keeps climbing** — a
  general bug, not a kickoff bug, but it shows up here because the intros are
  thread replies. Observed repeatedly, including in the 2026-07-18 run. The cause
  of the *asymmetry* is confirmed: the count and the list read from different
  sources. The count is the relay's server-computed recount pushed as kind 39005
  (`hooks.ts`, `channelWindowStore.ts` `mergeLiveThreadSummary`) — authoritative
  and independent of the reply event itself. The list is a client-maintained cache
  (`["thread-replies", channelId, rootId]`) appended to by parsing live events.
  One is robust, the other is fragile, so they can disagree.

  The specific trigger is **not** confirmed. Leading hypothesis: a refetch
  overwrite race in `useThreadReplies.ts` — the query is `staleTime: 0`, and on
  refetch it replaces the cache with the server's answer while preserving only
  replies that arrived *during* the fetch (`receivedInFlight` is diffed against
  `idsAtStart`). A live reply that landed just *before* a refetch, and that the
  server's answer doesn't yet include, is dropped. Welcome refetches unusually
  often because there are two observers on that cache entry (the pane's, plus
  `welcomeKickoff.ts`'s opener-subtree watch). Fits every symptom, including why
  close+reopen fixes it (the later fetch catches up). Ruled out for this case:
  malformed `e` tags (the CLI/SDK emit proper NIP-10 markers) and root-key
  divergence (the opener *is* the NIP-10 root).

  Note the live-append branch that feeds the pane has **no test coverage at all**,
  which is why this can regress quietly. Needs a repro test before a fix — don't
  fix on the hypothesis.
