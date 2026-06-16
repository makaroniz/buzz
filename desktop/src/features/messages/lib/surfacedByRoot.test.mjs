import assert from "node:assert/strict";
import test from "node:test";

import { buildSurfacedByRoot } from "./surfacedByRoot.mjs";

const group = (id, rootId, count) => ({
  message: { id, rootId, body: "x", author: "a" },
  count,
});

test("keys each collapsed reply by its thread root id", () => {
  const map = buildSurfacedByRoot([
    group("nested1", "rootA", 3),
    group("nested2", "rootB", 1),
  ]);
  assert.equal(map.size, 2);
  assert.equal(map.get("rootA").count, 3);
  assert.equal(map.get("rootB").count, 1);
});

test("a reply with no rootId keys by its own id (it is its own thread root)", () => {
  const map = buildSurfacedByRoot([group("solo", null, 1)]);
  assert.equal(map.get("solo").count, 1);
});

test("a root entry id that is NOT a surfaced thread key yields no pill (undefined)", () => {
  // The renderer attaches via map.get(entry.message.id). An entry whose id is
  // not a surfaced thread key gets nothing — the common "this root has no
  // buried replies to me" case.
  const map = buildSurfacedByRoot([group("nested", "rootA", 2)]);
  assert.equal(map.get("some-other-root"), undefined);
});

test("broadcast-root / off-window edge: a key matching no entry yields no pill, no throw", () => {
  // A surfaced thread whose root id is absent from the rendered entries (the
  // root scrolled out, or a broadcast-rooted subthread whose marker points at a
  // non-entry). The renderer looks up by ENTRY id, so this key is simply never
  // queried -> no pill, no orphan, no throw. Modeled here: the key exists in the
  // map but no entry id will ever equal it.
  const map = buildSurfacedByRoot([group("nested", "absent-root", 5)]);
  const entryIds = new Set(["visibleRootA", "visibleRootB"]);
  const pills = [...entryIds].map((id) => map.get(id)).filter(Boolean);
  assert.deepEqual(pills, []);
  // The orphan key is present in the map but never resolves to a rendered pill.
  assert.equal(entryIds.has("absent-root"), false);
});
