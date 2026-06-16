/**
 * Pure collapse of surfaced replies into one pointer per thread. `surfaceReplies`
 * yields every buried agent->viewer reply individually; in this team's threading
 * model a single thread can hold dozens, which floods the timeline with pointer
 * rows and destroys its structure. This collapses all surfaced replies sharing a
 * thread (`rootId ?? id`) into ONE representative carrying the group's count.
 *
 * The representative is the MOST RECENT surfaced reply in the thread (max
 * `createdAt`, `id` as a deterministic tiebreak). The downstream merge sorts the
 * pointer by that representative's `createdAt`, so the collapsed pill lands where
 * the newest buried activity is; clicking it navigates to that same most-recent
 * reply (the freshest thing addressed to the viewer).
 *
 * Lives in `.mjs` (not `.ts`) so the TS-loader-less test runner imports the same
 * source production uses; the sibling `.d.mts` types the result for TS callers.
 */

/**
 * @param {{ id: string, createdAt: number, rootId?: string | null }[]} surfaced
 * @returns {{ message: object, count: number }[]} one entry per thread; `message`
 *   is the most-recent surfaced reply, `count` the number collapsed into it.
 */
export function collapseSurfacedReplies(surfaced) {
  /** @type {Map<string, { message: object, count: number }>} */
  const byThread = new Map();

  for (const message of surfaced) {
    const threadId = message.rootId ?? message.id;
    const group = byThread.get(threadId);
    if (!group) {
      byThread.set(threadId, { message, count: 1 });
      continue;
    }
    group.count += 1;
    if (isMoreRecent(message, group.message)) group.message = message;
  }

  return [...byThread.values()];
}

/** Most recent = greater `createdAt`, breaking ties on greater `id`. */
function isMoreRecent(candidate, current) {
  if (candidate.createdAt !== current.createdAt) {
    return candidate.createdAt > current.createdAt;
  }
  return candidate.id > current.id;
}
