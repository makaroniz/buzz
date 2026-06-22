import assert from "node:assert/strict";
import test from "node:test";

import {
  respondToUpdateForReusedAgent,
  runtimeUpdateForReusedAgent,
} from "./channelAgents.ts";

const PUBKEY = "a".repeat(64);

function agent(overrides = {}) {
  return {
    id: "agent-1",
    pubkey: PUBKEY,
    name: "Reusable",
    personaId: "persona-1",
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    avatarUrl: null,
    model: null,
    mcpToolsets: null,
    envVars: {},
    status: "running",
    pid: null,
    createdAt: new Date(0).toISOString(),
    updatedAt: new Date(0).toISOString(),
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("respondToUpdateForReusedAgent resets omitted mode to owner-only", () => {
  assert.deepEqual(
    respondToUpdateForReusedAgent(
      agent({
        respondTo: "anyone",
        respondToAllowlist: [PUBKEY],
      }),
      {},
    ),
    {
      respondTo: "owner-only",
      respondToAllowlist: [],
    },
  );
});

test("respondToUpdateForReusedAgent leaves matching owner-only agents unchanged", () => {
  assert.equal(respondToUpdateForReusedAgent(agent(), {}), null);
});

test("respondToUpdateForReusedAgent carries explicit allowlist choices", () => {
  assert.deepEqual(
    respondToUpdateForReusedAgent(agent(), {
      respondTo: "allowlist",
      respondToAllowlist: [PUBKEY],
    }),
    {
      respondTo: "allowlist",
      respondToAllowlist: [PUBKEY],
    },
  );
});

test("runtimeUpdateForReusedAgent leaves matching runtime unchanged", () => {
  assert.equal(
    runtimeUpdateForReusedAgent(agent({ agentArgs: ["acp"] }), {
      id: "goose",
      label: "Goose",
      command: "goose",
      defaultArgs: ["acp"],
      mcpCommand: null,
    }),
    null,
  );
});

test("runtimeUpdateForReusedAgent returns command fields for runtime overrides", () => {
  assert.deepEqual(
    runtimeUpdateForReusedAgent(agent({ agentArgs: ["acp"] }), {
      id: "claude",
      label: "Claude Code",
      command: "claude-acp",
      defaultArgs: ["--mode", "acp"],
      mcpCommand: "claude-mcp",
    }),
    {
      agentCommand: "claude-acp",
      agentArgs: ["--mode", "acp"],
      mcpCommand: "claude-mcp",
    },
  );
});
