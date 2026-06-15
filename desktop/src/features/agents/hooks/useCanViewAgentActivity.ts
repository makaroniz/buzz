import { useQuery } from "@tanstack/react-query";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { resolveCanViewAgentActivity } from "@/features/agents/lib/canViewAgentActivity";
import { resolveAgentOwnership } from "@/shared/api/tauriAgentOwnership";

export const agentOwnershipQueryKey = (agentPubkey: string) =>
  ["agentOwnership", agentPubkey.toLowerCase()] as const;

export function useAgentOwnershipQuery(
  agentPubkey: string | null | undefined,
  enabled = true,
) {
  return useQuery({
    enabled: enabled && Boolean(agentPubkey),
    queryKey: agentOwnershipQueryKey(agentPubkey ?? ""),
    queryFn: () => resolveAgentOwnership(agentPubkey as string),
    staleTime: 60_000,
  });
}

/**
 * Relay-authoritative gate for observer activity visibility.
 *
 * Returns `{ canView, isLoading }`. While ownership is loading, locally
 * managed agents may show activity optimistically; the final answer always
 * comes from relay `is_agent_owner`.
 */
export function useCanViewAgentActivity(
  agentPubkey: string | null | undefined,
  options?: { enabled?: boolean },
) {
  const enabled = (options?.enabled ?? true) && Boolean(agentPubkey);
  const ownershipQuery = useAgentOwnershipQuery(agentPubkey, enabled);
  const isManagedAgent = useIsManagedAgent(enabled ? agentPubkey : null);

  return resolveCanViewAgentActivity({
    relayOwnership: ownershipQuery.data,
    isManagedAgent,
    isOwnershipLoading: ownershipQuery.isLoading,
    isManagedLoading: isManagedAgent === undefined,
  });
}
