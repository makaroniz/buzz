import { maxReadAt } from "@/features/channels/readState/readStateFormat";

export type ObservedUnreadEvent = {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  countsTowardBadge: boolean;
  countsTowardAppBadge: boolean;
};

export function makeObservedUnreadEvent(input: {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  channelType: string | undefined;
  isThreadedReply: boolean;
}): ObservedUnreadEvent {
  const isDm = input.channelType === "dm";
  return {
    id: input.id,
    createdAt: input.createdAt,
    rootId: input.rootId,
    highPriority: input.highPriority,
    countsTowardBadge: isDm || input.isThreadedReply || input.highPriority,
    countsTowardAppBadge:
      isDm || (!input.isThreadedReply && input.highPriority),
  };
}

export function mapsEqual(
  a: ReadonlyMap<string, number>,
  b: ReadonlyMap<string, number>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [key, value] of a) {
    if (b.get(key) !== value) return false;
  }
  return true;
}

export function recordObservedUnreadEvent(
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
  channelId: string,
  event: ObservedUnreadEvent,
  limit: number,
): boolean {
  let eventsById = eventsByChannel.get(channelId);
  if (!eventsById) {
    eventsById = new Map<string, ObservedUnreadEvent>();
    eventsByChannel.set(channelId, eventsById);
  }
  if (eventsById.has(event.id)) return false;

  eventsById.set(event.id, event);
  if (eventsById.size <= limit) return true;

  const oldest = [...eventsById.values()].sort(
    (a, b) => a.createdAt - b.createdAt,
  )[0]?.id;
  if (oldest) {
    eventsById.delete(oldest);
  }
  return true;
}

// Drop observed refs for channels whose unread events the read markers now
// fully cover. `markThreadRead`/`markMessageRead` advance a `thread:<root>`/
// `msg:<id>` marker, but `markChannelRead`'s clearObserved prune keys these maps
// by that synthetic key — never the real channel — so the real channel's refs
// linger after a thread/message read covers its last badge event. A channel
// absent from `unreadChannelIds` (and not forced or active) has no unread
// observed events left, so its refs are dead weight. Pruning them is invisible
// to the count (a covered channel and an absent one both contribute 0).
export function pruneCoveredObservedRefs(
  latestByChannel: Map<string, number>,
  observedByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
  channelIds: Iterable<string>,
  unreadChannelIds: ReadonlySet<string>,
  forcedChannelIds: ReadonlySet<string>,
  activeChannelId: string | null,
): void {
  for (const channelId of channelIds) {
    if (channelId === activeChannelId) continue;
    if (unreadChannelIds.has(channelId)) continue;
    if (forcedChannelIds.has(channelId)) continue;
    if (latestByChannel.get(channelId) === undefined) continue;
    latestByChannel.delete(channelId);
    observedByChannel.delete(channelId);
  }
}

export function countUnreadObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function countUnreadBadgeObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.countsTowardBadge) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function countUnreadAppBadgeObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.countsTowardAppBadge) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function countUnreadHighPriorityObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.highPriority) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function observedUnreadEventReadAt(
  event: ObservedUnreadEvent,
  channelReadAt: number | null,
  getThreadOwnMarker: (rootId: string) => number | null,
  getMessageOwnMarker: (messageId: string) => number | null = () => null,
): number | null {
  const markers = [channelReadAt, getMessageOwnMarker(event.id)];

  if (event.rootId !== null) {
    markers.push(getThreadOwnMarker(event.rootId));
  }

  return maxReadAt(...markers);
}
