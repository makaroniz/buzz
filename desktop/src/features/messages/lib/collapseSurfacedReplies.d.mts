/**
 * Type declarations for the pure collapse in `collapseSurfacedReplies.mjs`.
 * Runtime lives in `.mjs` so the (TS-loader-less) `node:test` runner imports it
 * directly; this file gives TypeScript callers a typed view.
 */
import type { TimelineMessage } from "@/features/messages/types";

/** One collapsed pointer per thread: the most-recent reply plus its group size. */
export type CollapsedSurfacedReply = {
  message: TimelineMessage;
  count: number;
};

/**
 * Collapses surfaced replies sharing a thread (`rootId ?? id`) into one entry
 * per thread, keeping the most-recent reply (max `createdAt`, `id` tiebreak) as
 * the representative and counting the group.
 */
export function collapseSurfacedReplies(
  surfaced: TimelineMessage[],
): CollapsedSurfacedReply[];
