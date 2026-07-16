import assert from "node:assert/strict";
import test from "node:test";

import { mergeKnownAgentPubkeys } from "./knownAgentPubkeys.ts";

const MANAGED =
  "1111111111111111111111111111111111111111111111111111111111111111";
const RELAY =
  "2222222222222222222222222222222222222222222222222222222222222222";

test("mergesTrustedSources", () => {
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED }],
    [{ pubkey: RELAY }],
  );

  assert.deepEqual([...merged].sort(), [MANAGED, RELAY].sort());
});

test("undefinedSources_yieldEmptySet", () => {
  const merged = mergeKnownAgentPubkeys(undefined, undefined);

  assert.equal(merged.size, 0);
});

test("normalisesCaseAndWhitespace_dedupingAcrossSources", () => {
  // The same agent appearing in multiple sources with different casing /
  // stray whitespace must collapse to one normalised entry, so membership
  // checks against normalizePubkey output always hit.
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED.toUpperCase() }],
    [{ pubkey: ` ${MANAGED}` }],
  );

  assert.deepEqual([...merged], [MANAGED]);
});

// ── relay scoping ────────────────────────────────────────────────────────────

const RELAY_A = "ws://relay-a.example.com:3000";
const RELAY_B = "wss://relay-b.example.com";

test("managedAgentPinnedToOtherRelay_excludedWhenScoped", () => {
  // With multiple communities' agents running concurrently, a managed agent
  // pinned to relay B is not an agent "in" community A — it must not enter
  // A's known-agent baseline just because it's locally managed.
  const merged = mergeKnownAgentPubkeys(
    [
      { pubkey: MANAGED, relayUrl: RELAY_B },
      // Cosmetic URL differences must not split an agent from its community.
      { pubkey: RELAY, relayUrl: `${RELAY_A}/` },
    ],
    undefined,
    RELAY_A,
  );

  assert.deepEqual([...merged], [RELAY]);
});

test("blankPinnedRelay_followsActiveCommunity", () => {
  // Defense-in-depth mirror of the backend's blank-relay fallback: a record
  // that escaped stamping belongs to whichever community is visited.
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED, relayUrl: "" }],
    undefined,
    RELAY_A,
  );

  assert.deepEqual([...merged], [MANAGED]);
});

test("relayAgents_neverRelayFiltered", () => {
  // Relay agents come from the active relay's own kind:10100 profiles —
  // already community-scoped at the source, so the merge must keep them
  // even when a managed-agent scope is in effect.
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED, relayUrl: RELAY_B }],
    [{ pubkey: RELAY }],
    RELAY_A,
  );

  assert.deepEqual([...merged], [RELAY]);
});

test("noActiveRelay_degradesToUnscopedMerge", () => {
  const merged = mergeKnownAgentPubkeys(
    [{ pubkey: MANAGED, relayUrl: RELAY_B }],
    undefined,
    null,
  );

  assert.deepEqual([...merged], [MANAGED]);
});
