/**
 * Type declarations for the pure attach-map construction in `surfacedByRoot.mjs`.
 * Runtime lives in `.mjs` so the (TS-loader-less) `node:test` runner imports it
 * directly; this file gives TypeScript callers a typed view.
 */
import type { CollapsedSurfacedReply } from "@/features/messages/lib/collapseSurfacedReplies";

/**
 * Builds the thread-root-id -> collapsed-reply attach map. The renderer looks up
 * `get(rootEntry.message.id)`; a key matching no entry renders no pill (no orphan).
 */
export function buildSurfacedByRoot(
  collapsed: CollapsedSurfacedReply[],
): Map<string, CollapsedSurfacedReply>;
