import type { ManagedAgent, ManagedAgentBackend } from "@/shared/api/types";

/** Inline normalization — avoids runtime dependency on @/shared/lib/pubkey. */
function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function commandBasename(command: string) {
  const normalized = command.trim().replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] ?? normalized;
}

function normalizeCommandIdentity(command: string) {
  const lower = commandBasename(command).toLowerCase();
  if (lower === "claude-code-acp" || lower === "claude-agent-acp") {
    return "claude-acp";
  }
  return lower;
}

export function commandsMatch(left: string, right: string) {
  return normalizeCommandIdentity(left) === normalizeCommandIdentity(right);
}

export function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return 0;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

export function pickPreferredManagedAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const leftRunningScore =
      left.status === "running" || left.status === "deployed" ? 1 : 0;
    const rightRunningScore =
      right.status === "running" || right.status === "deployed" ? 1 : 0;
    if (leftRunningScore !== rightRunningScore) {
      return rightRunningScore - leftRunningScore;
    }

    return parseTimestamp(right.updatedAt) - parseTimestamp(left.updatedAt);
  })[0];
}

export function findReusablePersonaAgent(
  agents: ManagedAgent[],
  personaId: string,
): ManagedAgent | undefined {
  const candidates = agents.filter((agent) => agent.personaId === personaId);
  return pickPreferredManagedAgent(candidates);
}

export type ReusablePersonaAgentRequest = {
  personaId: string;
  command: string;
  defaultArgs: readonly string[];
  mcpCommand?: string | null;
  backend?: ManagedAgentBackend;
};

export function findReusablePersonaAgentForRequest(
  agents: ManagedAgent[],
  request: ReusablePersonaAgentRequest,
): ManagedAgent | undefined {
  const candidates = agents.filter((agent) =>
    reusablePersonaAgentMatchesRequest(agent, request),
  );
  return pickPreferredManagedAgent(candidates);
}

export function reusablePersonaAgentMatchesRequest(
  agent: ManagedAgent,
  request: ReusablePersonaAgentRequest,
): boolean {
  return (
    agent.personaId === request.personaId &&
    managedAgentRuntimeMatchesRequest(agent, request) &&
    managedAgentBackendMatchesRequest(agent.backend, request.backend)
  );
}

export function managedAgentRuntimeMatchesRequest(
  agent: ManagedAgent,
  request: Pick<
    ReusablePersonaAgentRequest,
    "command" | "defaultArgs" | "mcpCommand"
  >,
) {
  return (
    commandsMatch(agent.agentCommand, request.command) &&
    stringArraysEqual(agent.agentArgs, request.defaultArgs) &&
    normalizeOptionalCommand(agent.mcpCommand) ===
      normalizeOptionalCommand(request.mcpCommand)
  );
}

export function managedAgentBackendMatchesRequest(
  existing: ManagedAgentBackend,
  requested: ManagedAgentBackend | undefined,
) {
  const normalizedRequested = requested ?? { type: "local" as const };
  if (existing.type !== normalizedRequested.type) return false;
  if (existing.type === "local") return true;
  if (normalizedRequested.type !== "provider") return false;

  return (
    existing.id === normalizedRequested.id &&
    stableStringify(existing.config) ===
      stableStringify(normalizedRequested.config)
  );
}

export function findReusableGenericAgent(
  agents: ManagedAgent[],
  command: string,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  const candidates = agents.filter(
    (agent) =>
      !agent.personaId &&
      !agent.systemPrompt?.trim() &&
      commandsMatch(agent.agentCommand, command) &&
      !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(candidates);
}

/**
 * Check if a reusable agent exists for the given input. Used by the UI to
 * surface the "reuse vs create new" guardrail before submission.
 */
export function findReusableAgent(
  agents: ManagedAgent[],
  channelMemberPubkeys: ReadonlySet<string>,
  input: {
    personaId?: string | null;
    systemPrompt?: string;
    command: string;
  },
): ManagedAgent | undefined {
  if (input.personaId) {
    return findReusablePersonaAgent(agents, input.personaId);
  }
  if (!input.systemPrompt?.trim()) {
    return findReusableGenericAgent(
      agents,
      input.command,
      channelMemberPubkeys,
    );
  }
  return undefined;
}

function stringArraysEqual(left: readonly string[], right: readonly string[]) {
  if (left.length !== right.length) return false;
  return left.every((value, index) => value === right[index]);
}

function normalizeOptionalCommand(value: string | null | undefined) {
  return value?.trim() ?? "";
}

function stableStringify(value: unknown): string {
  return JSON.stringify(sortJsonLikeValue(value));
}

function sortJsonLikeValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJsonLikeValue);
  }
  if (!value || typeof value !== "object") {
    return value;
  }

  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, entry]) => [key, sortJsonLikeValue(entry)]),
  );
}
