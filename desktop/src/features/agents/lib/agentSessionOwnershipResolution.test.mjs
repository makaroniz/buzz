import assert from "node:assert/strict";
import test from "node:test";

import {
  getChannelAgentSessionAgents,
  resolveOpenAgentSessionAgent,
} from "../../channels/lib/agentSessionCandidates.ts";

const agent = (pubkey, source) => ({
  pubkey,
  name: pubkey.slice(0, 8),
  status: "deployed",
  agentSource: source,
  canInterruptTurn: source === "managed",
});

test("resolveOpenAgentSessionAgent prefers channel-scoped candidate", () => {
  const channelAgent = agent("aa".repeat(32), "managed");
  const otherAgent = agent("bb".repeat(32), "relay");

  const resolved = resolveOpenAgentSessionAgent({
    allAgentCandidates: [channelAgent, otherAgent],
    channelAgentSessionAgents: [channelAgent],
    openAgentSessionPubkey: channelAgent.pubkey,
  });

  assert.equal(resolved, channelAgent);
});

test("resolveOpenAgentSessionAgent falls back to owned agent outside channel list", () => {
  const ownedAgent = agent("cc".repeat(32), "relay");

  const resolved = resolveOpenAgentSessionAgent({
    allAgentCandidates: [ownedAgent],
    channelAgentSessionAgents: [],
    openAgentSessionPubkey: ownedAgent.pubkey,
  });

  assert.equal(resolved, ownedAgent);
});

test("resolveOpenAgentSessionAgent synthesizes minimal agent when metadata is stale", () => {
  const pubkey = "dd".repeat(32);

  const resolved = resolveOpenAgentSessionAgent({
    allAgentCandidates: [],
    channelAgentSessionAgents: [],
    openAgentSessionPubkey: pubkey,
  });

  assert.deepEqual(resolved, {
    pubkey,
    name: pubkey.slice(0, 8),
    status: "deployed",
    agentSource: "relay",
    canInterruptTurn: false,
  });
});

test("getChannelAgentSessionAgents keeps managed agents visible in channel membership", () => {
  const activeChannel = {
    id: "channel-1",
    name: "general",
  };
  const candidates = [agent("ee".repeat(32), "managed")];

  const visible = getChannelAgentSessionAgents({
    activeChannel,
    activeChannelId: activeChannel.id,
    agents: candidates,
    channelMembers: [
      {
        pubkey: candidates[0].pubkey,
        role: "bot",
        displayName: "Scout",
      },
    ],
  });

  assert.equal(visible.length, 1);
  assert.equal(visible[0]?.pubkey, candidates[0].pubkey);
});
