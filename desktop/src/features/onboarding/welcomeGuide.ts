import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
} from "@/shared/api/tauri";
import { sendManagedAgentChannelMessage } from "@/shared/api/tauriManagedAgentMessages";
import { listPersonas, setPersonaActive } from "@/shared/api/tauriPersonas";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const WELCOME_GUIDE_AGENT_NAME = "Brain";
export const WELCOME_GUIDE_PERSONA_ID = "builtin:brain";
export const WELCOME_WORKER_AGENT_NAME = "Brawn";
export const WELCOME_WORKER_PERSONA_ID = "builtin:brawn";
export const WELCOME_GUIDE_INTRO_MARKER = "buzz-welcome-intro.v1";
const LEGACY_WELCOME_GUIDE_AGENT_NAME = "Kit";
export const LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT =
  "You are Kit, Sprout's friendly welcome guide. Help new users understand the community, channels, messages, and agents. Keep introductions concise, practical, and warm.";
export const WELCOME_GUIDE_INTRO_MESSAGE =
  "Hi, I'm Brain. Welcome to Buzz.\n\nI focus on research and planning, and Brawn focuses on implementation and validation. Ask either of us for help, or bring us a goal and we'll work through it together.";

const WELCOME_AGENT_DEFINITIONS = [
  {
    name: WELCOME_GUIDE_AGENT_NAME,
    personaId: WELCOME_GUIDE_PERSONA_ID,
  },
  {
    name: WELCOME_WORKER_AGENT_NAME,
    personaId: WELCOME_WORKER_PERSONA_ID,
  },
] as const;

type WelcomeAgentDefinition = (typeof WELCOME_AGENT_DEFINITIONS)[number];

function normalizeRelayUrl(relayUrl: string | null | undefined) {
  return relayUrl?.trim().replace(/\/+$/, "") ?? null;
}

function isAgentScopedToRelay(agent: ManagedAgent, relayUrl?: string | null) {
  const targetRelayUrl = normalizeRelayUrl(relayUrl);
  if (!targetRelayUrl) {
    return true;
  }
  return normalizeRelayUrl(agent.relayUrl) === targetRelayUrl;
}

function isNamedAgent(agent: ManagedAgent, name: string) {
  return agent.name.trim().toLowerCase() === name.toLowerCase();
}

function isBuiltInWelcomeAgent(
  agent: ManagedAgent,
  definition: WelcomeAgentDefinition,
) {
  return (
    agent.personaId === definition.personaId &&
    isNamedAgent(agent, definition.name)
  );
}

function isBuiltInWelcomeGuideAgent(agent: ManagedAgent) {
  return isBuiltInWelcomeAgent(agent, WELCOME_AGENT_DEFINITIONS[0]);
}

function isLegacyKitWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    agent.name.trim().toLowerCase() ===
      LEGACY_WELCOME_GUIDE_AGENT_NAME.toLowerCase() &&
    agent.systemPrompt?.trim() === LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT
  );
}

function isWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    isBuiltInWelcomeGuideAgent(agent) || isLegacyKitWelcomeGuideAgent(agent)
  );
}

function isWelcomeAgent(agent: ManagedAgent) {
  return (
    WELCOME_AGENT_DEFINITIONS.some((definition) =>
      isBuiltInWelcomeAgent(agent, definition),
    ) || isLegacyKitWelcomeGuideAgent(agent)
  );
}

function pickAgentByStatus(agents: ManagedAgent[]) {
  return (
    agents.find((agent) => agent.status === "running") ??
    agents.find((agent) => agent.status === "deployed") ??
    agents[0] ??
    null
  );
}

export function pickWelcomeGuideAgent(agents: ManagedAgent[]) {
  return pickAgentByStatus(agents.filter(isWelcomeGuideAgent));
}

export function pickWelcomeGuideAgentForRelay(
  agents: ManagedAgent[],
  relayUrl?: string | null,
) {
  return pickAgentByStatus(
    agents.filter(
      (agent) =>
        isWelcomeGuideAgent(agent) && isAgentScopedToRelay(agent, relayUrl),
    ),
  );
}

export async function getWelcomeAgentPubkeys(relayUrl?: string | null) {
  return (await listManagedAgents())
    .filter(
      (agent) => isWelcomeAgent(agent) && isAgentScopedToRelay(agent, relayUrl),
    )
    .map((agent) => agent.pubkey);
}

async function ensureWelcomePersonaActive(definition: WelcomeAgentDefinition) {
  const persona = (await listPersonas()).find(
    (candidate) => candidate.id === definition.personaId,
  );
  if (!persona) {
    throw new Error(`${definition.name} agent not found.`);
  }
  if (!persona.isActive) {
    await setPersonaActive(definition.personaId, true);
  }
}

async function ensureWelcomeAgent(
  definition: WelcomeAgentDefinition,
  relayUrl?: string | null,
) {
  const agents = await listManagedAgents();
  const existing = pickAgentByStatus(
    agents.filter(
      (agent) =>
        isBuiltInWelcomeAgent(agent, definition) &&
        isAgentScopedToRelay(agent, relayUrl),
    ),
  );
  if (existing) {
    return existing;
  }

  await ensureWelcomePersonaActive(definition);

  const created = await createManagedAgent({
    name: definition.name,
    personaId: definition.personaId,
    relayUrl: relayUrl ?? undefined,
    spawnAfterCreate: false,
    startOnAppLaunch: false,
    respondTo: "owner-only",
  });

  return created.agent;
}

async function ensureWelcomeGuideMembership(
  channelId: string,
  agent: ManagedAgent,
) {
  const agentPubkey = normalizePubkey(agent.pubkey);
  const members = await getChannelMembers(channelId).catch(() => []);
  if (
    members.some((member) => normalizePubkey(member.pubkey) === agentPubkey)
  ) {
    return;
  }

  const result = await addChannelMembers({
    channelId,
    pubkeys: [agent.pubkey],
    role: "bot",
  });
  const error = result.errors.find(
    (entry) => normalizePubkey(entry.pubkey) === agentPubkey,
  );
  if (error && !error.error.toLowerCase().includes("already")) {
    throw new Error(error.error);
  }
}

export async function ensureWelcomeGuideIntro(
  channelId: string,
  relayUrl?: string | null,
) {
  const guide = await ensureWelcomeAgent(
    WELCOME_AGENT_DEFINITIONS[0],
    relayUrl,
  );
  const worker = await ensureWelcomeAgent(
    WELCOME_AGENT_DEFINITIONS[1],
    relayUrl,
  );
  await ensureWelcomeGuideMembership(channelId, guide);
  await ensureWelcomeGuideMembership(channelId, worker);
  await sendManagedAgentChannelMessage({
    agentPubkey: guide.pubkey,
    channelId,
    content: WELCOME_GUIDE_INTRO_MESSAGE,
    marker: WELCOME_GUIDE_INTRO_MARKER,
    markerScope: "channel",
  });
  return guide;
}
