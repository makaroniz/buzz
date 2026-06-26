import assert from "node:assert/strict";
import test from "node:test";

import {
  createPersonaDialogState,
  duplicatePersonaDialogState,
  editPersonaDialogState,
  importPersonaDialogState,
  saveAsPersonaTemplateDialogState,
} from "./personaDialogState.ts";

test("createPersonaDialogState returns a fresh empty draft", () => {
  const first = createPersonaDialogState();
  const second = createPersonaDialogState();

  assert.equal(first.title, "Create persona");
  assert.deepEqual(first.initialValues, {
    displayName: "",
    avatarUrl: "",
    systemPrompt: "",
    runtime: undefined,
    model: undefined,
  });
  assert.notStrictEqual(first.initialValues, second.initialValues);
});

test("duplicatePersonaDialogState copies persona fields into a new draft", () => {
  const state = duplicatePersonaDialogState({
    id: "persona-1",
    displayName: "Solo",
    avatarUrl: "avatar://solo",
    systemPrompt: "Be direct.",
    runtime: "provider-a",
    model: "model-a",
    provider: null,
    isBuiltIn: false,
    isActive: true,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues, {
    displayName: "Solo copy",
    avatarUrl: "avatar://solo",
    systemPrompt: "Be direct.",
    runtime: "provider-a",
    model: "model-a",
    provider: undefined,
    namePool: [],
    envVars: {},
  });
});

test("duplicatePersonaDialogState carries envVars and namePool into the duplicate", () => {
  // Regression: codex R10 P2. Without this, a duplicated persona that
  // relies on an API key in env_vars would silently fail at spawn until
  // the user re-entered every credential.
  const state = duplicatePersonaDialogState({
    id: "persona-with-secrets",
    displayName: "Coder",
    avatarUrl: null,
    systemPrompt: "Write code.",
    runtime: null,
    model: null,
    isBuiltIn: false,
    isActive: true,
    namePool: ["alice", "bob"],
    envVars: { ANTHROPIC_API_KEY: "sk-test", GOOSE_PROVIDER: "anthropic" },
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues.envVars, {
    ANTHROPIC_API_KEY: "sk-test",
    GOOSE_PROVIDER: "anthropic",
  });
  assert.deepEqual(state.initialValues.namePool, ["alice", "bob"]);
});

test("editPersonaDialogState preserves the persona id for updates", () => {
  const state = editPersonaDialogState({
    id: "persona-2",
    displayName: "Kit",
    avatarUrl: null,
    systemPrompt: "Keep it weird.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: true,
    isActive: true,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.title, "Edit persona");
  assert.equal(state.description, "");
  assert.equal(state.submitLabel, "Save changes");
  assert.deepEqual(state.initialValues, {
    id: "persona-2",
    displayName: "Kit",
    avatarUrl: "",
    systemPrompt: "Keep it weird.",
    runtime: undefined,
    model: undefined,
    provider: undefined,
    namePool: [],
    envVars: {},
  });
});

test("editPersonaDialogState seeds envVars and namePool from the persona", () => {
  const state = editPersonaDialogState({
    id: "persona-3",
    displayName: "Coder",
    avatarUrl: null,
    systemPrompt: "Write code.",
    runtime: null,
    model: null,
    isBuiltIn: false,
    isActive: true,
    namePool: ["alice", "bob"],
    envVars: { ANTHROPIC_API_KEY: "sk-test" },
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues.envVars, {
    ANTHROPIC_API_KEY: "sk-test",
  });
  assert.deepEqual(state.initialValues.namePool, ["alice", "bob"]);
});

test("importPersonaDialogState maps parsed persona previews into create drafts", () => {
  const state = importPersonaDialogState({
    displayName: "Imported",
    avatarDataUrl: null,
    systemPrompt: "Imported prompt",
    runtime: null,
    model: "model-b",
    provider: null,
    namePool: [],
    sourceFile: "import.persona.json",
  });

  assert.equal(state.title, "Import Imported");
  assert.deepEqual(state.initialValues, {
    displayName: "Imported",
    avatarUrl: "",
    systemPrompt: "Imported prompt",
    runtime: undefined,
    model: "model-b",
    provider: undefined,
  });
});

// ── Provider round-trip tests ─────────────────────────────────────────────────

test("editPersonaDialogState preserves provider=databricks", () => {
  const state = editPersonaDialogState({
    id: "persona-provider",
    displayName: "DB Agent",
    avatarUrl: null,
    systemPrompt: "Use databricks.",
    runtime: "goose",
    model: "dbrx",
    provider: "databricks",
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, "databricks");
});

test("editPersonaDialogState maps provider=null to undefined", () => {
  const state = editPersonaDialogState({
    id: "persona-no-provider",
    displayName: "Plain",
    avatarUrl: null,
    systemPrompt: "No provider.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, undefined);
});

test("duplicatePersonaDialogState preserves provider=databricks", () => {
  const state = duplicatePersonaDialogState({
    id: "persona-dup-provider",
    displayName: "DB Agent",
    avatarUrl: null,
    systemPrompt: "Use databricks.",
    runtime: "goose",
    model: "dbrx",
    provider: "databricks",
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, "databricks");
});

test("importPersonaDialogState preserves provider=anthropic", () => {
  const state = importPersonaDialogState({
    displayName: "Imported With Provider",
    avatarDataUrl: null,
    systemPrompt: "Anthropic agent.",
    runtime: "goose",
    model: "claude-sonnet",
    provider: "anthropic",
    namePool: [],
    sourceFile: "provider-test.persona.json",
  });

  assert.equal(state.initialValues.provider, "anthropic");
});

// ── saveAsPersonaTemplateDialogState (promote an agent → persona template) ────

/** Minimal ManagedAgent fixture; only the fields the builder reads matter. */
function makeAgent(overrides = {}) {
  return {
    pubkey: "npub-agent",
    name: "Scout",
    personaId: null,
    relayUrl: "wss://relay",
    acpCommand: "",
    agentCommand: "/usr/local/bin/goose-acp",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 24,
    systemPrompt: "Scout the codebase.",
    model: "claude-sonnet",
    mcpToolsets: null,
    envVars: { ANTHROPIC_API_KEY: "sk-test" },
    status: "stopped",
    pid: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    ...overrides,
  };
}

test("saveAsPersonaTemplateDialogState uses persona-template vocabulary", () => {
  const state = saveAsPersonaTemplateDialogState(makeAgent(), []);

  assert.equal(state.title, "Save as persona template");
  assert.equal(state.submitLabel, "Save as persona template");
  assert.equal(state.description, "Reuse this setup to create more agents.");
});

test("saveAsPersonaTemplateDialogState reverse-maps agentCommand to a runtime id", () => {
  // commandsMatch compares basenames, so the absolute agentCommand path
  // resolves to the catalog runtime whose command shares that basename.
  const state = saveAsPersonaTemplateDialogState(makeAgent(), [
    { id: "claude", label: "Claude", command: "claude-code-acp" },
    { id: "goose", label: "Goose", command: "goose-acp" },
  ]);

  assert.equal(state.initialValues.runtime, "goose");
});

test("saveAsPersonaTemplateDialogState leaves runtime undefined when nothing matches", () => {
  // Graceful fallback: an unknown command (or an empty/unloaded catalog)
  // yields no runtime, and the dialog falls back to its default behavior.
  const noMatch = saveAsPersonaTemplateDialogState(makeAgent(), [
    { id: "claude", label: "Claude", command: "claude-code-acp" },
  ]);
  const empty = saveAsPersonaTemplateDialogState(makeAgent(), []);

  assert.equal(noMatch.initialValues.runtime, undefined);
  assert.equal(empty.initialValues.runtime, undefined);
});

test("saveAsPersonaTemplateDialogState ignores catalog entries with a null command", () => {
  const state = saveAsPersonaTemplateDialogState(makeAgent(), [
    { id: "not-installed", label: "Not Installed", command: null },
    { id: "goose", label: "Goose", command: "goose-acp" },
  ]);

  assert.equal(state.initialValues.runtime, "goose");
});

test("saveAsPersonaTemplateDialogState carries name, prompt, model, and envVars", () => {
  const state = saveAsPersonaTemplateDialogState(makeAgent(), []);

  assert.equal(state.initialValues.displayName, "Scout");
  assert.equal(state.initialValues.systemPrompt, "Scout the codebase.");
  assert.equal(state.initialValues.model, "claude-sonnet");
  assert.deepEqual(state.initialValues.envVars, {
    ANTHROPIC_API_KEY: "sk-test",
  });
  // namePool is persona-only and starts empty for the user to fill.
  assert.deepEqual(state.initialValues.namePool, []);
});

test("saveAsPersonaTemplateDialogState omits provider (no top-level field on ManagedAgent)", () => {
  // A ManagedAgent has no top-level provider, so there is nothing lossless to
  // carry — the builder must not invent one.
  const state = saveAsPersonaTemplateDialogState(makeAgent(), []);

  assert.equal("provider" in state.initialValues, false);
});

test("saveAsPersonaTemplateDialogState tolerates null systemPrompt and model", () => {
  const state = saveAsPersonaTemplateDialogState(
    makeAgent({ systemPrompt: null, model: null }),
    [],
  );

  assert.equal(state.initialValues.systemPrompt, "");
  assert.equal(state.initialValues.model, undefined);
});
