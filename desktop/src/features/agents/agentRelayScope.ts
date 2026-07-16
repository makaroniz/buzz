/**
 * Relay-scoping for managed agents. Every managed-agent record is pinned to
 * its home relay (`ManagedAgent.relayUrl`, stamped by the backend at create),
 * and the desktop UI presents agents per community — so every surface that
 * lists, counts, or acts on managed agents must scope to the active
 * community's relay through these helpers. Agents pinned to other relays
 * keep running in the background; they are just not "in" this community.
 */

/**
 * Canonical form of a relay URL for identity comparisons — NOT for
 * connecting. Frontend mirror of `normalize_relay_url` in
 * `desktop/src-tauri/src/relay.rs`; the two must agree because record pins
 * are stamped by the backend and compared here: trim, strip trailing
 * slashes, lowercase scheme + authority (case-insensitive per RFC 3986),
 * preserve any path or query case-sensitively.
 *
 * Distinct from `normalizeRelayUrl` in `communityStorage.ts` (input
 * canonicalisation: prepends `wss://`) and in `selfProfileStorage.ts`
 * (storage keys: lowercases the whole URL, path included).
 */
export function normalizeRelayUrlForCompare(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "");
  const schemeEnd = trimmed.indexOf("://");
  if (schemeEnd === -1) {
    return trimmed;
  }
  const scheme = trimmed.slice(0, schemeEnd);
  const rest = trimmed.slice(schemeEnd + "://".length);
  const pathStart = rest.indexOf("/");
  const authority = pathStart === -1 ? rest : rest.slice(0, pathStart);
  const path = pathStart === -1 ? "" : rest.slice(pathStart);
  return `${scheme.toLowerCase()}://${authority.toLowerCase()}${path}`;
}

/**
 * Whether an agent record belongs to the given community relay.
 *
 * A blank pin follows the active community — the same defense-in-depth
 * fallback as the backend's `effective_agent_relay_url` for records that
 * escaped stamping. A blank/absent community relay (no provider, no active
 * community yet) degrades to unscoped rather than blanking every surface.
 */
export function agentBelongsToRelay(
  agentRelayUrl: string | null | undefined,
  communityRelayUrl: string | null | undefined,
): boolean {
  const community = communityRelayUrl?.trim() ?? "";
  if (community === "") {
    return true;
  }
  const pinned = agentRelayUrl?.trim() ?? "";
  if (pinned === "") {
    return true;
  }
  return (
    normalizeRelayUrlForCompare(pinned) ===
    normalizeRelayUrlForCompare(community)
  );
}

/**
 * Split agents into those pinned to the active community's relay and the
 * rest. `inCommunity` drives the agents screen; `other` exists only for
 * cross-community affordances (the "running in other communities" count).
 */
export function partitionAgentsByRelay<T extends { relayUrl?: string | null }>(
  agents: readonly T[] | undefined,
  communityRelayUrl: string | null | undefined,
): { inCommunity: T[]; other: T[] } {
  const inCommunity: T[] = [];
  const other: T[] = [];
  for (const agent of agents ?? []) {
    if (agentBelongsToRelay(agent.relayUrl, communityRelayUrl)) {
      inCommunity.push(agent);
    } else {
      other.push(agent);
    }
  }
  return { inCommunity, other };
}

/**
 * The managed-agents polling gate: poll only while an agent *in this
 * community* is running. Agents running in other communities don't render
 * process state on this community's surfaces, so they must not keep its
 * 5s liveness poll alive.
 */
export function hasRunningAgentInCommunity(
  agents: readonly { relayUrl?: string | null; status: string }[] | undefined,
  communityRelayUrl: string | null | undefined,
): boolean {
  return (agents ?? []).some(
    (agent) =>
      agent.status === "running" &&
      agentBelongsToRelay(agent.relayUrl, communityRelayUrl),
  );
}

/**
 * Whether any local agent process is running, in ANY community. Drives the
 * slow cross-community poll tier: all-communities surfaces (the community
 * rail's active-agents dot, the "running in other communities" line) render
 * last-known process state for backgrounded communities, so a relaxed poll
 * must keep that state from going permanently stale when a background
 * process dies. Deployed (provider-backed) agents are excluded — their
 * status changes only through control-plane actions, never silently.
 */
export function hasRunningAgentAnywhere(
  agents: readonly { status: string }[] | undefined,
): boolean {
  return (agents ?? []).some((agent) => agent.status === "running");
}

/**
 * Active (running or deployed — mirrors `isManagedAgentActive`) managed
 * agents per community id, for surfaces that render ALL communities at once
 * (the community rail).
 *
 * A pinned agent counts toward every community whose relay normalizes to
 * its pin — two rail entries pointing at the same relay genuinely share
 * their agents. A blank pin counts toward the ACTIVE community only: the
 * per-surface `agentBelongsToRelay` fallback ("blank follows the community
 * being viewed") would light every community for one stray unstamped
 * record when evaluated against all of them side by side.
 */
export function countActiveAgentsByCommunity(
  agents: readonly { relayUrl?: string | null; status: string }[] | undefined,
  communities: readonly { id: string; relayUrl: string }[],
  activeCommunityId: string | null,
): Map<string, number> {
  const counts = new Map<string, number>();
  const idsByRelay = new Map<string, string[]>();
  for (const community of communities) {
    const key = normalizeRelayUrlForCompare(community.relayUrl);
    const ids = idsByRelay.get(key);
    if (ids) {
      ids.push(community.id);
    } else {
      idsByRelay.set(key, [community.id]);
    }
  }
  for (const agent of agents ?? []) {
    if (agent.status !== "running" && agent.status !== "deployed") {
      continue;
    }
    const pinned = agent.relayUrl?.trim() ?? "";
    const ids =
      pinned === ""
        ? activeCommunityId !== null
          ? [activeCommunityId]
          : []
        : (idsByRelay.get(normalizeRelayUrlForCompare(pinned)) ?? []);
    for (const id of ids) {
      counts.set(id, (counts.get(id) ?? 0) + 1);
    }
  }
  return counts;
}
