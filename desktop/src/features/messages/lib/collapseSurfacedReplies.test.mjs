import assert from "node:assert/strict";
import test from "node:test";

// Imports the exact source the renderer wires. No inlined copy -> no drift
// between test expectations and production behaviour.
import { collapseSurfacedReplies } from "./collapseSurfacedReplies.mjs";

// Minimal shape: collapse reads id, createdAt, rootId. A nested reply carries
// its thread root's id; a reply with no rootId is its own thread.
const reply = (id, createdAt, rootId) => ({ id, createdAt, rootId });

// Reduce to a comparable trace of (representative id, count).
const trace = (groups) =>
  groups.map((g) => `${g.message.id}:${g.count}`).sort();

test("replies in one thread collapse to a single representative carrying the count", () => {
  const out = collapseSurfacedReplies([
    reply("a", 10, "root"),
    reply("b", 30, "root"),
    reply("c", 20, "root"),
  ]);
  // Most recent (createdAt 30) represents; count is the group size.
  assert.deepEqual(trace(out), ["b:3"]);
});

test("two threads collapse independently with no cross-thread merge", () => {
  const out = collapseSurfacedReplies([
    reply("a1", 10, "rootA"),
    reply("a2", 40, "rootA"),
    reply("b1", 20, "rootB"),
    reply("b2", 30, "rootB"),
  ]);
  // rootA -> a2 (max 40, count 2); rootB -> b2 (max 30, count 2).
  assert.deepEqual(trace(out), ["a2:2", "b2:2"]);
});

test("representative is the max createdAt, tiebroken on greater id", () => {
  // Two replies share createdAt 30; the greater id ("y" > "x") wins.
  const out = collapseSurfacedReplies([
    reply("x", 30, "root"),
    reply("y", 30, "root"),
    reply("w", 10, "root"),
  ]);
  assert.deepEqual(trace(out), ["y:3"]);
});

test("a reply with no rootId is its own thread keyed by its id", () => {
  const out = collapseSurfacedReplies([reply("solo", 10, null)]);
  assert.deepEqual(trace(out), ["solo:1"]);
});

test("a root-level reply and a nested reply under it share one thread", () => {
  // The thread root's own thread id is its id; a nested reply carries rootId
  // equal to that id -> same thread key -> one collapsed pointer.
  const out = collapseSurfacedReplies([
    reply("root", 10, null),
    reply("nested", 20, "root"),
  ]);
  assert.deepEqual(trace(out), ["nested:2"]);
});

test("empty input returns empty", () => {
  assert.deepEqual(collapseSurfacedReplies([]), []);
});
