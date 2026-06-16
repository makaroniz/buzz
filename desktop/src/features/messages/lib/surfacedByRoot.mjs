/**
 * Pure construction of the surfaced-pill attach map: thread root id -> the
 * collapsed representative reply + count. The timeline renders only root
 * entries and looks up `surfacedByRoot.get(entry.message.id)` to attach a pill
 * below each root. Keying by the thread root id (`rootId ?? id`) and driving the
 * lookup from the ENTRY side is the no-orphan contract: a surfaced thread whose
 * root is not a rendered entry (off-window, or a broadcast-rooted subthread
 * whose NIP-10 root marker points at a different id than the on-screen broadcast
 * entry) simply never gets looked up, so it renders no pill rather than a
 * detached orphan.
 *
 * Lives in `.mjs` (not `.ts`) so the TS-loader-less test runner imports the same
 * source production uses; the sibling `.d.mts` types it for TypeScript callers.
 */

/**
 * @param {{ message: { id: string, rootId?: string | null }, count: number }[]} collapsed
 * @returns {Map<string, { message: object, count: number }>} keyed by thread root id
 */
export function buildSurfacedByRoot(collapsed) {
  return new Map(
    collapsed.map((group) => [group.message.rootId ?? group.message.id, group]),
  );
}
