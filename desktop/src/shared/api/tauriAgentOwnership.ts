import { invokeTauri } from "@/shared/api/tauri";

export type AgentOwnershipStatus = {
  /** Lowercase hex pubkey of the queried agent. */
  agentPubkey: string;
  /** Lowercase hex owner pubkey from relay `agent_owner_pubkey`, if set. */
  ownerPubkey: string | null;
  /** True iff the current workspace identity is the relay-recorded owner. */
  isOwner: boolean;
};

type RawAgentOwnershipStatus = {
  agent_pubkey: string;
  owner_pubkey: string | null;
  is_owner: boolean;
};

/**
 * Resolve whether the current identity owns `agentPubkey` per relay DB.
 * Authoritative gate for observer activity visibility.
 */
export async function resolveAgentOwnership(
  agentPubkey: string,
): Promise<AgentOwnershipStatus> {
  const raw = await invokeTauri<RawAgentOwnershipStatus>(
    "resolve_agent_ownership",
    { agentPubkey },
  );
  return {
    agentPubkey: raw.agent_pubkey,
    ownerPubkey: raw.owner_pubkey,
    isOwner: raw.is_owner,
  };
}
