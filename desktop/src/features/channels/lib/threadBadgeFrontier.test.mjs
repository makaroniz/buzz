import assert from "node:assert/strict";
import test from "node:test";

import { nextThreadBadgeFrontier } from "./threadBadgeFrontier.ts";
import { seedThreadBadgeFrontiers } from "./threadBadgeFrontier.ts";
import { buildRepliesByRootId } from "./subtreeCreatedAt.ts";
import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";

const msg = (id, parentId) => ({ id, parentId, rootId: parentId ?? id });
const seedAll = () => true;
const seed = (frontiers, messages, isNotified, getReadAt) =>
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    buildRepliesByRootId(messages),
    isNotified,
    getReadAt,
  );

test("nextThreadBadgeFrontier_unseededNullMarker_seedsNull", () => {
  // Thread never read: snapshot seeds to null (everything unread).
  assert.equal(nextThreadBadgeFrontier(undefined, null), null);
});

test("nextThreadBadgeFrontier_unseededWithMarker_seedsToMarker", () => {
  assert.equal(nextThreadBadgeFrontier(undefined, 100), 100);
});

test("nextThreadBadgeFrontier_readAdvancesMarker_advancesSnapshot", () => {
  // Snapshot frozen at open (null), user reads → live marker 200 → badge clears.
  assert.equal(nextThreadBadgeFrontier(null, 200), 200);
});

test("nextThreadBadgeFrontier_markerNewerThanStored_advances", () => {
  assert.equal(nextThreadBadgeFrontier(100, 250), 250);
});

test("nextThreadBadgeFrontier_markerOlderThanStored_keepsStored", () => {
  // Monotonic: a stale lower marker never lowers the snapshot.
  assert.equal(nextThreadBadgeFrontier(250, 100), 250);
});

test("nextThreadBadgeFrontier_markerNullAfterSeed_keepsStored", () => {
  // Live marker reads null (never read) but snapshot already advanced — hold.
  assert.equal(nextThreadBadgeFrontier(150, null), 150);
});

test("nextThreadBadgeFrontier_markerEqualsStored_unchanged", () => {
  assert.equal(nextThreadBadgeFrontier(150, 150), 150);
});

test("nextThreadBadgeFrontier_storedNullMarkerZero_advancesToZero", () => {
  // Zero is a valid frontier (epoch); null is strictly lower than any number.
  assert.equal(nextThreadBadgeFrontier(null, 0), 0);
});

test("seedThreadBadgeFrontiers_threadWithReplies_seedsToMarker", () => {
  const frontiers = new Map();
  const messages = [msg("root", null), msg("r1", "root")];
  seed(frontiers, messages, seedAll, (id) => (id === "root" ? 100 : null));
  assert.equal(frontiers.get("root"), 100);
});

test("seedThreadBadgeFrontiers_threadWithoutReplies_skipped", () => {
  const frontiers = new Map();
  seed(frontiers, [msg("root", null)], seedAll, () => 100);
  assert.equal(frontiers.has("root"), false);
});

test("seedThreadBadgeFrontiers_notNotified_skipped", () => {
  const frontiers = new Map();
  const messages = [msg("root", null), msg("r1", "root")];
  seed(
    frontiers,
    messages,
    () => false,
    () => 100,
  );
  assert.equal(frontiers.has("root"), false);
});

test("seedThreadBadgeFrontiers_replyEntry_neverSeeded", () => {
  // A reply is never a badge root even if its id collides with a notified set.
  const frontiers = new Map();
  const messages = [msg("r1", "root"), msg("r2", "root")];
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.size, 0);
});

test("seedThreadBadgeFrontiers_reseed_advancesMonotonically", () => {
  const frontiers = new Map([["root", 100]]);
  const messages = [msg("root", null), msg("r1", "root")];
  // Re-render after the live marker advanced to 250 on read.
  seed(frontiers, messages, seedAll, () => 250);
  assert.equal(frontiers.get("root"), 250);
  // A stale lower marker never lowers an already-advanced snapshot.
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 250);
});

// --- LP4 Case 3, seed-timing face: channel marker must not bleed into seed ---
//
// seedThreadBadgeFrontiers seeds each root via getReadAt(root). AppShell's
// getThreadReadAt(rootId, channelId?) returns the thread's OWN marker when no
// channelId is passed, but max(thread_own, channel) when one is — the NIP-RS
// hierarchical fold. Channel-open markChannelRead advances the channel marker
// to the newest top-level message; if the seed reads the FOLDED marker, that
// fresh channel timestamp seeds the frontier PAST an unread reply and the badge
// vanishes everywhere (computeThreadBadgeCounts then reads zero). The seed is
// monotonic (nextThreadBadgeFrontier uses Math.max), so once the channel marker
// has advanced, any re-render re-bleeds it back — the flaky "no badge" the user
// saw correlated with newer channel messages.
//
// The fix: the seed reads the thread's OWN marker (getThreadReadAt(root) with no
// channelId), never the folded marker. Thread badges are channel-independent by
// design (NIP-RS Option 1: channel-open leaves them intact until each thread is
// read); the own marker advances only when the thread itself is read, which is
// exactly the advance-on-read the monotonic seed wants.
//
// These tests model getThreadReadAt exactly as AppShell defines it and drive
// seed -> compute end-to-end. The folded path is the regression tripwire (badge
// vanishes); the own-marker path is the fixed behavior (badge survives).

// Richer message shape than the file-level `msg`: computeThreadBadgeCounts reads
// createdAt and pubkey, which the frontier-only helper omits. rootId defaults to
// the parent (getThreadReference's fallback) so these fixtures stay falsifiable
// once the seed/count pipeline re-keys on rootId.
const reply = (id, parentId, createdAt, rootId) => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
  pubkey: "author",
});

// Faithful model of AppShell's getThreadReadAt(rootId, channelId?): own marker
// alone, or folded with the channel marker via Math.max when a channelId is
// passed. The seed must invoke this WITHOUT a channelId.
const makeGetThreadReadAt = (threadOwn, channel) => (_rootId, channelId) => {
  if (channelId == null) return threadOwn;
  if (threadOwn === null) return channel;
  if (channel === null) return threadOwn;
  return Math.max(threadOwn, channel);
};

const seedAndCount = (messages, getReadAt) => {
  const repliesByRootId = buildRepliesByRootId(messages);
  const frontiers = new Map();
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    repliesByRootId,
    seedAll,
    (id) => getReadAt(id),
  );
  return {
    frontier: frontiers.get("root"),
    counts: computeThreadBadgeCounts(
      messages,
      repliesByRootId,
      frontiers,
      seedAll,
    ),
  };
};

test("seedThreadBadgeFrontiers_channelMarkerFoldedIntoSeed_badgeVanishes", () => {
  // Thread "root" has one unread reply at 200. Own marker is 100 (reply is
  // genuinely unread), but channel-open advanced the channel marker to 250.
  // Seeding via the FOLDED accessor (channelId passed) reads max(100, 250)=250.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const getThreadReadAt = makeGetThreadReadAt(100, 250);

  const { frontier, counts } = seedAndCount(messages, (id) =>
    getThreadReadAt(id, "channel-1"),
  );
  // Regression tripwire: folded marker seeds past the unread reply -> no badge.
  assert.equal(frontier, 250);
  assert.equal(counts.has("root"), false);
});

test("seedThreadBadgeFrontiers_ownMarkerSeeded_badgeSurvives", () => {
  // Identical thread, but the seed reads the thread's OWN marker (no channelId),
  // so the channel fold never applies — the fixed wiring.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const getThreadReadAt = makeGetThreadReadAt(100, 250);

  const { frontier, counts } = seedAndCount(messages, (id) =>
    getThreadReadAt(id),
  );
  // Fixed: frontier seeded to the own marker (100), behind the unread reply.
  assert.equal(frontier, 100);
  assert.equal(counts.get("root"), 1);
});

test("seedThreadBadgeFrontiers_threadReadNewerThanChannel_noSpuriousShift", () => {
  // Edge case: the THREAD was read more recently (180) than the channel (120),
  // and one reply at 200 is still unread. The own marker and the folded marker
  // happen to coincide here (max(180, 120) = 180), so both paths agree the reply
  // at 200 is unread — the own-marker path must not under- or over-clear it.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const getThreadReadAt = makeGetThreadReadAt(180, 120);

  const own = seedAndCount(messages, (id) => getThreadReadAt(id));
  assert.equal(own.frontier, 180);
  assert.equal(own.counts.get("root"), 1);
});

test("seedThreadBadgeFrontiers_ownMarkerCoversReply_badgeClears", () => {
  // The thread itself was read past the reply (own marker 250 >= reply at 200),
  // so the badge correctly clears regardless of the channel marker. Confirms the
  // own-marker seed still advances-on-read — it isn't pinned at "always unread".
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const getThreadReadAt = makeGetThreadReadAt(250, 120);

  const { frontier, counts } = seedAndCount(messages, (id) =>
    getThreadReadAt(id),
  );
  assert.equal(frontier, 250);
  assert.equal(counts.has("root"), false);
});

// --- LP4 Case 3, second face: orphan-only root IS seed-eligible ---
//
// seedThreadBadgeFrontiers gates seed-eligibility on a reply existing under the
// root by rootId: `if (!repliesByRootId.has(message.id)) continue;`. A root
// whose ONLY reply is a deep orphan — the middle ancestor unloaded, so the
// orphan keys under its absent parent in the direct-reply map — still owns that
// reply by rootId (getThreadReference resolves rootId from the event's own
// `root` e-tag regardless of ancestor load state). The old direct-reply gate
// skipped such a root entirely, so its frontier never existed and its badge
// could never clear; keying on rootId makes it seed-eligible. This is a face of
// Case 3 distinct from a wrong COUNT — here the frontier snapshot itself was
// missing, so even a corrected count path had nothing to measure against.

test("seedThreadBadgeFrontiers_orphanOnlyRoot_seedEligible", () => {
  // root's only reply is `c`, whose middle ancestor `b` is unloaded. `c` keys
  // under "b" (absent) in the direct-reply map but carries rootId === "root",
  // so repliesByRootId has an entry for "root" and the root seeds to marker 100.
  const messages = [
    reply("root", null, 50),
    // reply("b", "root", ...) — intentionally absent: unloaded ancestor.
    reply("c", "b", 200, "root"),
  ];
  const frontiers = new Map();
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 100);
});

test("seedThreadBadgeFrontiers_directReplyRoot_seedEligible_DESIRED", () => {
  // Control: the SAME root with a DIRECT reply present. repliesByRootId has an
  // entry for "root", so it is seed-eligible — the intact-chain baseline.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const frontiers = new Map();
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 100);
});
