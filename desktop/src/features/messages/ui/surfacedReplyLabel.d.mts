/**
 * Type declarations for the pure label derivation in `surfacedReplyLabel.mjs`.
 * Runtime lives in `.mjs` so the (TS-loader-less) `node:test` runner imports it
 * directly; this file gives TypeScript callers a typed view.
 */

/** Single-line, truncated preview of a reply body; null when empty/whitespace-only. */
export function deriveSnippet(body: string | null | undefined): string | null;

/** Presentational parts of a surfaced-reply pill. */
export type SurfacedReplyLabel = {
  /** Single-line truncated body preview, or null to render the no-snippet idiom. */
  snippet: string | null;
  /** "N replies" suffix for N>1, or null for a single reply. */
  countSuffix: string | null;
};

export function surfacedReplyLabel(args: {
  body: string | null | undefined;
  count: number;
}): SurfacedReplyLabel;
