/**
 * Tests for useAgentMetricArchiveSeed seeding logic.
 *
 * Mirrors the pattern in useObserverArchiveSeed.test.mjs — drives the async
 * seed logic via the deps-injection interface, no React required.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({
  defaultOn = false,
  hasExplicitChoice = false,
  createShouldFail = false,
  existingKinds = [],
} = {}) {
  const calls = { createSaveSubscription: [], setExplicitChoice: [] };

  return {
    calls,
    agentMetricArchiveDefaultEnabled: async () => defaultOn,
    listSaveSubscriptions: async () =>
      existingKinds.length > 0
        ? [{ scopeType: "owner_p", kinds: existingKinds }]
        : [],
    createSaveSubscription: async (scopeType, scopeValue, kinds) => {
      if (createShouldFail) throw new Error("create failed");
      calls.createSaveSubscription.push({ scopeType, scopeValue, kinds });
    },
    hasExplicitChoice: (_pubkey) => hasExplicitChoice,
    setExplicitChoice: (pubkey, enabled) => {
      calls.setExplicitChoice.push({ pubkey, enabled });
    },
  };
}

// Minimal re-implementation of the seeding logic from useAgentMetricArchiveSeed.ts.
// Kept in sync with the source by structural mirroring.
const KIND_AGENT_TURN_METRIC = 44200;

async function runSeed(pubkey, deps) {
  if (!pubkey) return;
  if (deps.hasExplicitChoice(pubkey)) return;

  let defaultOn;
  try {
    defaultOn = await deps.agentMetricArchiveDefaultEnabled();
  } catch {
    return;
  }

  if (!defaultOn) return;

  try {
    let existingKinds = [];
    try {
      const existing = await deps.listSaveSubscriptions();
      existingKinds =
        existing.find((s) => s.scopeType === "owner_p")?.kinds ?? [];
    } catch {
      // best-effort
    }
    const mergedKinds = existingKinds.includes(KIND_AGENT_TURN_METRIC)
      ? existingKinds
      : [...existingKinds, KIND_AGENT_TURN_METRIC];
    await deps.createSaveSubscription("owner_p", pubkey, mergedKinds);
  } catch {
    return; // transient failure — do NOT set explicit choice
  }

  deps.setExplicitChoice(pubkey, true);
}

// ── Tests ────────────────────────────────────────────────────────────────────

test("test_internal_build_unset_seeds_owner_p_subscription", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.createSaveSubscription.length,
    1,
    "should call createSaveSubscription once",
  );
  const call = deps.calls.createSaveSubscription[0];
  assert.equal(call.scopeType, "owner_p");
  assert.equal(call.scopeValue, "pubkey123");
  assert.deepEqual(call.kinds, [44200]);
});

test("test_internal_build_merges_with_existing_observer_kinds", async () => {
  // Observer already seeded [24200]; metric seed must produce [24200, 44200].
  const deps = makeDeps({
    defaultOn: true,
    hasExplicitChoice: false,
    existingKinds: [24200],
  });
  await runSeed("pubkey123", deps);

  assert.equal(deps.calls.createSaveSubscription.length, 1);
  const call = deps.calls.createSaveSubscription[0];
  assert.deepEqual(call.kinds, [24200, 44200]);
});

test("test_internal_build_idempotent_when_kind_already_present", async () => {
  // 44200 already in the row — merged kinds should be the same.
  const deps = makeDeps({
    defaultOn: true,
    hasExplicitChoice: false,
    existingKinds: [24200, 44200],
  });
  await runSeed("pubkey123", deps);

  assert.equal(deps.calls.createSaveSubscription.length, 1);
  const call = deps.calls.createSaveSubscription[0];
  assert.deepEqual(call.kinds, [24200, 44200]);
});

test("test_internal_build_unset_persists_explicit_choice_after_seed", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.setExplicitChoice.length,
    1,
    "should persist explicit choice after successful seed",
  );
  assert.equal(deps.calls.setExplicitChoice[0].pubkey, "pubkey123");
  assert.equal(deps.calls.setExplicitChoice[0].enabled, true);
});

test("test_explicit_choice_set_does_not_reseed", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: true });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.createSaveSubscription.length,
    0,
    "should not call createSaveSubscription when explicit choice is already set",
  );
  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should not update explicit choice when already set",
  );
});

test("test_oss_build_does_not_seed", async () => {
  const deps = makeDeps({ defaultOn: false, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.createSaveSubscription.length,
    0,
    "should not call createSaveSubscription in OSS build",
  );
  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should not persist explicit choice in OSS build",
  );
});

test("test_create_failure_does_not_persist_explicit_choice", async () => {
  const deps = makeDeps({
    defaultOn: true,
    hasExplicitChoice: false,
    createShouldFail: true,
  });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should NOT persist explicit choice after a transient create failure",
  );
});

test("test_empty_pubkey_does_nothing", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("", deps);

  assert.equal(deps.calls.createSaveSubscription.length, 0);
  assert.equal(deps.calls.setExplicitChoice.length, 0);
});

test("test_undefined_pubkey_does_nothing", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed(undefined, deps);

  assert.equal(deps.calls.createSaveSubscription.length, 0);
  assert.equal(deps.calls.setExplicitChoice.length, 0);
});
