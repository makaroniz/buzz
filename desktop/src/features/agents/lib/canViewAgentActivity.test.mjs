import assert from "node:assert/strict";
import test from "node:test";

import { resolveCanViewAgentActivity } from "./canViewAgentActivity.ts";

test("resolveCanViewAgentActivity returns true when relay confirms ownership", () => {
  const result = resolveCanViewAgentActivity({
    relayOwnership: {
      agentPubkey: "aa".repeat(32),
      ownerPubkey: "bb".repeat(32),
      isOwner: true,
    },
    isManagedAgent: false,
    isOwnershipLoading: false,
    isManagedLoading: false,
  });

  assert.equal(result.canView, true);
  assert.equal(result.isLoading, false);
});

test("resolveCanViewAgentActivity returns false when relay denies ownership", () => {
  const result = resolveCanViewAgentActivity({
    relayOwnership: {
      agentPubkey: "aa".repeat(32),
      ownerPubkey: "bb".repeat(32),
      isOwner: false,
    },
    isManagedAgent: true,
    isOwnershipLoading: false,
    isManagedLoading: false,
  });

  assert.equal(result.canView, false);
  assert.equal(result.isLoading, false);
});

test("resolveCanViewAgentActivity optimistically allows locally managed agents while loading", () => {
  const result = resolveCanViewAgentActivity({
    relayOwnership: undefined,
    isManagedAgent: true,
    isOwnershipLoading: true,
    isManagedLoading: false,
  });

  assert.equal(result.canView, true);
  assert.equal(result.isLoading, true);
});

test("resolveCanViewAgentActivity stays closed for non-managed agents while loading", () => {
  const result = resolveCanViewAgentActivity({
    relayOwnership: undefined,
    isManagedAgent: false,
    isOwnershipLoading: true,
    isManagedLoading: false,
  });

  assert.equal(result.canView, false);
  assert.equal(result.isLoading, true);
});
