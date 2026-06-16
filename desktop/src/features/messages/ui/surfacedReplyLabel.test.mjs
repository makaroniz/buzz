import assert from "node:assert/strict";
import test from "node:test";

import { deriveSnippet, surfacedReplyLabel } from "./surfacedReplyLabel.mjs";

test("snippet collapses whitespace and newlines to single spaces, trimmed", () => {
  assert.equal(
    deriveSnippet("  the   rebase\nis\t\tclean  "),
    "the rebase is clean",
  );
});

test("a short body is returned verbatim (after whitespace collapse)", () => {
  assert.equal(deriveSnippet("ready for your merge"), "ready for your merge");
});

test("empty body returns null (caller renders the no-snippet idiom)", () => {
  assert.equal(deriveSnippet(""), null);
});

test("whitespace-only body returns null", () => {
  assert.equal(deriveSnippet("   \n\t  "), null);
});

test("null/undefined body returns null", () => {
  assert.equal(deriveSnippet(null), null);
  assert.equal(deriveSnippet(undefined), null);
});

test("a long single-line body truncates to the budget with an ellipsis", () => {
  const body = "x".repeat(200);
  const out = deriveSnippet(body);
  // 72 chars + the single-char ellipsis.
  assert.equal(out.length, 73);
  assert.ok(out.endsWith("…"));
  assert.equal(out.slice(0, 72), "x".repeat(72));
});

test("truncation trims trailing space before the ellipsis (no '… ' gap)", () => {
  // 71 non-space chars then spaces then more text: cut at 72 lands mid-space.
  const body = `${"a".repeat(71)}   tail words here that overflow the budget`;
  const out = deriveSnippet(body);
  assert.ok(out.endsWith("…"));
  assert.ok(!out.includes(" …"), `no space before ellipsis: ${out}`);
});

test("body exactly at the budget is not truncated", () => {
  const body = "y".repeat(72);
  assert.equal(deriveSnippet(body), body);
});

test("label: count 1 yields a snippet and no count suffix", () => {
  const { snippet, countSuffix } = surfacedReplyLabel({
    body: "the deploy is green",
    count: 1,
  });
  assert.equal(snippet, "the deploy is green");
  assert.equal(countSuffix, null);
});

test("label: count > 1 demotes the count to an 'N replies' suffix after the snippet", () => {
  const { snippet, countSuffix } = surfacedReplyLabel({
    body: "first of several",
    count: 3,
  });
  assert.equal(snippet, "first of several");
  assert.equal(countSuffix, "3 replies");
});

test("label: count 1 never reads '1 replies'", () => {
  assert.equal(surfacedReplyLabel({ body: "hi", count: 1 }).countSuffix, null);
});

test("label: empty body with count > 1 keeps the suffix but drops the snippet", () => {
  const { snippet, countSuffix } = surfacedReplyLabel({ body: "", count: 4 });
  assert.equal(snippet, null);
  assert.equal(countSuffix, "4 replies");
});
