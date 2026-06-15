import type { AgentOwnershipStatus } from "@/shared/api/tauriAgentOwnership";

export type CanViewAgentActivityInput = {
  relayOwnership: AgentOwnershipStatus | undefined;
  isManagedAgent: boolean | undefined;
  isOwnershipLoading: boolean;
  isManagedLoading: boolean;
};

export type CanViewAgentActivityResult = {
  canView: boolean;
  isLoading: boolean;
};

/**
 * Unified predicate for Show Activity / Activity log ingresses.
 *
 * Final permission comes from relay `is_agent_owner`. While the relay lookup
 * is in flight, locally managed agents may show activity optimistically.
 */
export function resolveCanViewAgentActivity({
  relayOwnership,
  isManagedAgent,
  isOwnershipLoading,
  isManagedLoading,
}: CanViewAgentActivityInput): CanViewAgentActivityResult {
  if (relayOwnership?.isOwner === true) {
    return { canView: true, isLoading: false };
  }

  if (relayOwnership?.isOwner === false) {
    return { canView: false, isLoading: false };
  }

  const isLoading =
    isOwnershipLoading || (isManagedAgent === undefined && isManagedLoading);

  if (isManagedAgent === true && isOwnershipLoading) {
    return { canView: true, isLoading: true };
  }

  return { canView: false, isLoading };
}
