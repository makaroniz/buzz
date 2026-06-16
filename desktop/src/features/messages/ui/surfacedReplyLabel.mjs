/**
 * Pure derivation of the presentational parts of a surfaced-reply pill, so the
 * label logic is unit-testable without a DOM (the runner is `node --test` over
 * `.mjs`, no React renderer). `SurfacedReplyRow` renders these parts as:
 *
 *   {author} replied to you — "{snippet}" · {countSuffix}
 *
 * with the em-dash + quotes shown only when `snippet` is non-null and the
 * `· {countSuffix}` shown only when `countSuffix` is non-null.
 *
 * Lives in `.mjs` (not `.ts`) so the TS-loader-less test runner imports the same
 * source production uses; the sibling `.d.mts` types it for TypeScript callers.
 */

/** Max snippet length before truncation; chosen to fit one pill line. */
const SNIPPET_MAX = 72;
const ELLIPSIS = "…";

/**
 * Collapse a reply body to a single-line preview: newlines/runs of whitespace
 * become one space, trimmed, truncated to SNIPPET_MAX with an ellipsis. Returns
 * null for an empty/whitespace-only body so the caller renders the no-snippet
 * idiom rather than empty quotes.
 *
 * @param {string | null | undefined} body
 * @returns {string | null}
 */
export function deriveSnippet(body) {
  const oneLine = (body ?? "").replace(/\s+/g, " ").trim();
  if (oneLine === "") return null;
  if (oneLine.length <= SNIPPET_MAX) return oneLine;
  return oneLine.slice(0, SNIPPET_MAX).trimEnd() + ELLIPSIS;
}

/**
 * The two presentational parts of the pill. `countSuffix` is null for a single
 * reply (the headline already reads "replied to you") and `"N replies"` for N>1
 * so the count rides as a quiet suffix, never the headline. Count 1 never reads
 * "1 replies".
 *
 * @param {{ body: string | null | undefined, count: number }} args
 * @returns {{ snippet: string | null, countSuffix: string | null }}
 */
export function surfacedReplyLabel({ body, count }) {
  return {
    snippet: deriveSnippet(body),
    countSuffix: count > 1 ? `${count} replies` : null,
  };
}
