import { agentBelongsToRelay } from "@/features/agents/agentRelayScope";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Pure merge behind `useKnownAgentPubkeys`: managed agents ∪ relay agents,
 * normalised via `normalizePubkey` so membership checks work against
 * normalised pubkeys.
 *
 * Managed agents are scoped to `activeRelayUrl` (the active community's
 * relay): the baseline answers "is this pubkey an agent *in this
 * community*", and a locally managed agent pinned to another community's
 * relay is not — it neither posts here nor appears in this community's
 * directory. An agent genuinely present on both relays is still covered by
 * the relay-agent source (kind:10100 from the active relay), which is never
 * relay-filtered. Omitting `activeRelayUrl` degrades to the unscoped merge.
 *
 * Structurally typed on `{ pubkey, relayUrl? }` so node unit tests don't
 * need to build full `ManagedAgent`/`RelayAgent` values.
 */
export function mergeKnownAgentPubkeys(
  managedAgents:
    | readonly { pubkey: string; relayUrl?: string | null }[]
    | undefined,
  relayAgents: readonly { pubkey: string }[] | undefined,
  activeRelayUrl?: string | null,
): ReadonlySet<string> {
  const pubkeys = new Set<string>();
  for (const agent of managedAgents ?? []) {
    if (!agentBelongsToRelay(agent.relayUrl, activeRelayUrl)) continue;
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  for (const agent of relayAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  return pubkeys;
}
