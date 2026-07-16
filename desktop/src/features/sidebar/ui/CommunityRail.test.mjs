import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  communityRailIndicators,
  communityRailTooltipLabel,
} from "./CommunityRail.tsx";

describe("communityRailIndicators", () => {
  it("shows no badge for an observed community with unread but no mentions", () => {
    const r = communityRailIndicators({ hasUnread: true, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, true);
    assert.equal(r.pending, false);
  });

  it("shows no badge and no dot for an observed community with no unread", () => {
    const r = communityRailIndicators({ hasUnread: false, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.pending, false);
  });

  it("shows a mention badge with the count when mentions are present — no dot", () => {
    const r = communityRailIndicators({
      hasUnread: true,
      count: 3,
      state: "ready",
    });
    assert.equal(r.showBadge, true);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 3);
    assert.equal(r.badgeLabel, "3");
  });

  it("caps the badge label at 99+", () => {
    const r = communityRailIndicators({
      hasUnread: true,
      count: 250,
      state: "ready",
    });
    assert.equal(r.badgeLabel, "99+");
  });

  it("never reports mentions or dot for an unobserved (unknown) community", () => {
    const r = communityRailIndicators({
      hasUnread: true,
      count: 5,
      state: "unknown",
    });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 0);
    assert.equal(r.pending, true);
  });

  it("treats loading as pending — no badge, no dot", () => {
    const r = communityRailIndicators({ hasUnread: false, state: "loading" });
    assert.equal(r.pending, true);
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
  });

  it("never reports mentions or dot on an errored observation", () => {
    const r = communityRailIndicators({
      hasUnread: true,
      count: 2,
      state: "error",
    });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 0);
    assert.equal(r.pending, false);
  });
});

describe("communityRailTooltipLabel", () => {
  const quiet = { showBadge: false, showDot: false, mentionCount: 0 };

  it("is just the name with nothing to report", () => {
    assert.equal(communityRailTooltipLabel("Acme", quiet, 0), "Acme");
  });

  it("reports active agents with singular/plural forms", () => {
    assert.equal(
      communityRailTooltipLabel("Acme", quiet, 1),
      "Acme — 1 agent active",
    );
    assert.equal(
      communityRailTooltipLabel("Acme", quiet, 3),
      "Acme — 3 agents active",
    );
  });

  it("combines mentions with active agents — mentions beat plain unread", () => {
    assert.equal(
      communityRailTooltipLabel(
        "Acme",
        { showBadge: true, showDot: false, mentionCount: 2 },
        1,
      ),
      "Acme — 2 mentions, 1 agent active",
    );
  });

  it("combines plain unread with active agents", () => {
    assert.equal(
      communityRailTooltipLabel(
        "Acme",
        { showBadge: false, showDot: true, mentionCount: 0 },
        2,
      ),
      "Acme — unread, 2 agents active",
    );
  });

  it("keeps the plain unread form without agents", () => {
    assert.equal(
      communityRailTooltipLabel(
        "Acme",
        { showBadge: false, showDot: true, mentionCount: 0 },
        0,
      ),
      "Acme — unread",
    );
  });
});
