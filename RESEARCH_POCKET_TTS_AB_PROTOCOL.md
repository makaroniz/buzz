# Pocket TTS Quality — Blind A/B Listening Protocol

Owner: Sami. For PR `eva/pocket-tts-quality`. Pairs with Max's offline WAV corpus
harness. Goal: decide, with the ear and not the vibe, whether **FP32** and/or
**grouped chunking** actually beat our shipped **INT8 + per-sentence** path — and catch
any regression before changing the default users hear.

## What we're comparing (the conditions)

Max's harness renders every corpus item under the **2×2 matrix**, all other knobs held
at current values (temp 0.7, steps=1, speed removed/1.0, Mary voice, 24 kHz):

| Code | Precision | Chunking |
|------|-----------|----------|
| A    | int8      | per-sentence (current shipped) |
| B    | int8      | grouped ≤50 tok |
| C    | fp32      | per-sentence |
| D    | fp32      | grouped ≤50 tok |

A is the control (what ships today). Every judgment is "is X better than A, and is the
difference worth a 478 MB download (fp32) / a code change (grouping)?"

## Corpus (what text gets synthesized)

Every item rendered in all four conditions. Cases chosen to expose the artifact
families the research named, not generic prose.

1. **SHORT (1–4 words)** — cold-start smear surface; our 8-space pad only covers these.
   - `"Yep."` · `"On it."` · `"Sounds good to me."` · `"Done."`
2. **MULTI-SENTENCE (the seam case — the whole point of grouping)** — 3–5 sentences,
   the difference between A/C (per-sentence, 4 cold starts + 3 seams) and B/D (grouped,
   ~1 cold start, 0 interior seams) is maximal here.
   - `"I looked at the relay code this morning. The lease logic is solid. There's one
     race in the worker claim path, though. I'll write it up and send you a patch."`
   - `"Great question. The answer is it depends on the community size. For small ones,
     keep it simple."`
3. **MIXED real agent message** — punctuation, a number-to-words case, a clause break —
   representative of live huddle output.
   - `"That's 42 open PRs right now — mostly small. I'll triage them after lunch."`
4. **COLD-START** — first utterance of a fresh engine session (no prior warmup synth).
   Render item 1's `"Yep."` and item 2's first multi-sentence as the *very first*
   generate() call after engine load. Compare against the same text rendered warm.
5. **POST-IDLE (kyutai #91 — my flagged case)** — INT8 first-words garble after
   **prolonged dormancy**. Render a short + a multi-sentence item after **5 min** and
   **15 min** of engine idle. **This cannot be captured by a back-to-back offline loop**
   — see the harness note below. Most likely to differentiate int8 (A/B) from fp32 (C/D)
   if #91's dormancy-garble is precision-linked.

## Scoring — blind, rank-based

Absolute 1–10 scores per clip are noisy; humans rank reliably and score poorly. So per
corpus item the listener **ranks the 4 clips best→worst on overall naturalness**, then
flags specific artifacts. Blinding is mandatory.

### Blinding mechanics (Max's harness must implement)
- Filenames are opaque: `item03_clip1.wav … item03_clip4.wav`, where the clip→condition
  map is **randomized per item** and written to a `key.json` the listener does NOT open
  until scoring is submitted.
- Same corpus text within an item across the 4 clips (only precision/chunking differ).
- Loudness-match the 4 clips of an item to a common active-speech RMS target so louder
  ≠ "better" — a classic A/B confound. The harness only attenuates (to the quietest
  clip or a −23 dBFS ceiling), so it neither boosts/clips samples nor claims
  standards-compliant integrated LUFS normalization.

### Per-item scoring row (the sheet)
For each item, listener fills:
- **Rank**: order the 4 clip labels best→worst (ties allowed: `clip2 > clip1 = clip4 > clip3`).
- **Artifact flags** per clip (check any that apply) — these make the result diagnostic:
  - `seam` — audible discontinuity/jump between sentences (grouping should kill this)
  - `onset` — first syllable clipped/smeared/static
  - `garble` — hallucinated/wrong/mushy words (watch this on POST-IDLE int8)
  - `robotic` — flat/metallic/unnatural prosody
  - `timbre` — voice identity wobbles within the clip
  - `truncate` — cut off early / trailing words lost
- **Free note** (optional): one line, "clip3 breathed weirdly after the comma."

### Decision rule (pre-registered so we don't rationalize after)
- **Ship FP32** iff fp32 clips (C/D) rank above their int8 counterparts (A/B) on a
  majority of items AND the win shows up specifically on POST-IDLE/garble flags — i.e.
  #3172's claim reproduces on OUR path. If fp32 is merely "no worse," keep int8 (smaller
  download wins the tie).
- **Ship grouping** iff grouped clips (B/D) reduce `seam` flags on MULTI-SENTENCE items
  vs per-sentence (A/C) with no regression in `onset`/`garble`/time-to-first-audio.
  Grouping is a code change with no download cost, so "meaningfully fewer seams, nothing
  worse" is enough.
- **Neither is required to win.** A clean null ("A is fine") is a valid, PR-documented
  outcome — Tyler's "janky" may be an inherent zero-shot/tiny-model floor (Sami HN c.12,
  Václav c.94). Say so rather than shipping a change for its own sake.

## How Tyler listens (the practical part)

The protocol dies if it's a chore. Deliver Tyler:
1. A single folder of loudness-matched, opaquely-named WAVs grouped by item
   (`item01/clip1.wav…`), + a one-page scoring sheet (Markdown table, below) he fills in
   a text editor — no app, no build.
2. **Listen on the actual target output path if possible** — the huddle uses rodio at
   24 kHz mono through the OS device; laptop speakers are the real-world case, but ask
   him to also spot-check on headphones where seams/onset artifacts are most audible.
3. Suggested order: do all MULTI-SENTENCE items first (highest-signal for grouping),
   then SHORT, then COLD/IDLE. ~4 clips × ~8 items = 32 clips, ~15–20 min.
4. He submits the filled sheet; I un-blind with `key.json`, tally ranks + artifact-flag
   deltas per condition, and post the verdict + decision-rule outcome to the thread.

### Scoring sheet template (one block per item)
```
ITEM 02 (multi-sentence): "I looked at the relay code this morning. ..."
  rank (best→worst): ____ > ____ > ____ > ____
  clip1 flags: [ ]seam [ ]onset [ ]garble [ ]robotic [ ]timbre [ ]truncate   note:____
  clip2 flags: [ ]seam [ ]onset [ ]garble [ ]robotic [ ]timbre [ ]truncate   note:____
  clip3 flags: [ ]seam [ ]onset [ ]garble [ ]robotic [ ]timbre [ ]truncate   note:____
  clip4 flags: [ ]seam [ ]onset [ ]garble [ ]robotic [ ]timbre [ ]truncate   note:____
```

## Stateful idle capture
The **POST-IDLE cases (5/15 min) are stateful** — they need the engines loaded, then a
real wall-clock idle, then synthesis on the SAME warm-but-dormant engines. Run each
idle duration and corpus item separately so every captured clip is genuinely the first
synthesis after uninterrupted dormancy:

```bash
cargo run --release --manifest-path desktop/src-tauri/Cargo.toml \
  --example pocket_quality_ab -- \
  <int8-model-dir> <fp32-model-dir> <output-dir> \
  --idle-minutes 5 --only short_one_word
```

Repeat with `--idle-minutes 15`, and with `--only multi_relay_review`. Do not infer a
nil idle effect from the ordinary back-to-back corpus run. If these real waits are not
run, document idle-garble as **untested** rather than clean.
