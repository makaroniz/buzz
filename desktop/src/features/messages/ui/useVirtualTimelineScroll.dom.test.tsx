import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";

import type * as React from "react";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { Virtualizer } from "@tanstack/react-virtual";

import { buildVirtualTimelineRows } from "@/features/messages/lib/buildVirtualTimelineRows";
import type { TimelineMessage } from "@/features/messages/types";
import { useVirtualTimelineScroll } from "./useVirtualTimelineScroll";

afterEach(cleanup);

const DAY_1 = Math.floor(new Date(2026, 0, 1, 12, 0, 0).getTime() / 1000);

function message(
  overrides: Partial<TimelineMessage> & { id: string },
): TimelineMessage {
  return {
    createdAt: DAY_1,
    author: "tester",
    time: "",
    body: `body-${overrides.id}`,
    depth: 0,
    kind: 9,
    ...overrides,
  };
}

// Records scrollToIndex calls so we can assert the virtualizer is driven
// correctly. `getTotalSize` is controllable so we can simulate the document
// growing during first-load settle (estimate→measured heights, streaming rows).
function makeVirtualizer(initialTotalSize = 100) {
  let totalSize = initialTotalSize;
  const scrollToIndex =
    mock.fn<
      (index: number, opts?: { align?: string; behavior?: string }) => void
    >();
  return {
    scrollToIndex,
    setTotalSize(next: number) {
      totalSize = next;
    },
    virtualizer: {
      scrollToIndex,
      getTotalSize: () => totalSize,
    } as unknown as Virtualizer<HTMLDivElement, Element>,
  };
}

// A fake scroll container whose scroll metrics we control. `isNearBottom`
// reads scrollHeight/clientHeight/scrollTop; defaults (0) read as "at bottom".
function makeContainerRef(atBottom: boolean) {
  const el = {
    scrollHeight: atBottom ? 100 : 1000,
    clientHeight: 100,
    scrollTop: atBottom ? 0 : 0,
  } as unknown as HTMLDivElement;
  return { current: el } as React.RefObject<HTMLDivElement | null>;
}

test("on init with no deep-link target, scrolls to the last row (sticky bottom)", () => {
  const messages = [message({ id: "a" }), message({ id: "b" })];
  const rows = buildVirtualTimelineRows(messages);
  const { scrollToIndex, virtualizer } = makeVirtualizer();

  renderHook(() =>
    useVirtualTimelineScroll({
      channelId: "c1",
      isLoading: false,
      messages,
      rows,
      scrollContainerRef: makeContainerRef(true),
      virtualizer,
      scrollMarginReady: true,
    }),
  );

  // layout: [div, a, b] -> last index 2
  assert.ok(scrollToIndex.mock.calls.length >= 1);
  const [index, opts] = scrollToIndex.mock.calls[0].arguments;
  assert.equal(index, rows.length - 1);
  assert.equal(opts?.align, "end");
});

test("a new latest message while pinned autoscrolls; accent uses smooth", () => {
  const initial = [message({ id: "a" })];
  const rows1 = buildVirtualTimelineRows(initial);
  const { scrollToIndex, virtualizer } = makeVirtualizer();

  const { rerender } = renderHook(
    ({ messages, rows }) =>
      useVirtualTimelineScroll({
        channelId: "c1",
        isLoading: false,
        messages,
        rows,
        scrollContainerRef: makeContainerRef(true),
        virtualizer,
        scrollMarginReady: true,
      }),
    { initialProps: { messages: initial, rows: rows1 } },
  );

  scrollToIndex.mock.resetCalls();

  const next = [message({ id: "a" }), message({ id: "b", accent: true })];
  const rows2 = buildVirtualTimelineRows(next);
  act(() => {
    rerender({ messages: next, rows: rows2 });
  });

  assert.ok(scrollToIndex.mock.calls.length >= 1);
  const lastCall = scrollToIndex.mock.calls.at(-1);
  assert.equal(lastCall?.arguments[0], rows2.length - 1);
  assert.equal(lastCall?.arguments[1]?.behavior, "smooth");
});

test("a deep-link target scrolls to that message's flat row and centers it", () => {
  const messages = [
    message({ id: "a" }),
    message({ id: "b" }),
    message({ id: "c" }),
  ];
  const rows = buildVirtualTimelineRows(messages);
  const { scrollToIndex, virtualizer } = makeVirtualizer();
  const onTargetReached = mock.fn();

  renderHook(() =>
    useVirtualTimelineScroll({
      channelId: "c1",
      isLoading: false,
      messages,
      rows,
      scrollContainerRef: makeContainerRef(false),
      virtualizer,
      scrollMarginReady: true,
      targetMessageId: "b",
      onTargetReached,
    }),
  );

  // layout: [div, a, b, c] -> 'b' is flat index 2
  const centerCall = scrollToIndex.mock.calls.find(
    (call) => call.arguments[1]?.align === "center",
  );
  assert.ok(centerCall, "expected a centered scroll to the deep-link target");
  assert.equal(centerCall?.arguments[0], 2);
  assert.equal(onTargetReached.mock.calls.length, 1);
  assert.equal(onTargetReached.mock.calls[0].arguments[0], "b");
});

test("first-load settle: re-pins to bottom as total size grows while pinned", () => {
  // First load: rows paint with estimated heights, then measured heights /
  // streaming rows grow getTotalSize(). While pinned, each growth re-anchors to
  // the bottom so the viewport holds at the newest message.
  const messages = [message({ id: "a" }), message({ id: "b" })];
  const rows = buildVirtualTimelineRows(messages);
  const harness = makeVirtualizer(100);

  const { rerender } = renderHook(
    ({ totalSizeTick }: { totalSizeTick: number }) =>
      useVirtualTimelineScroll({
        channelId: "c1",
        isLoading: false,
        messages,
        rows,
        // Pinned at bottom (default scroll metrics read as at-bottom).
        scrollContainerRef: makeContainerRef(true),
        virtualizer: harness.virtualizer,
        scrollMarginReady: true,
        // totalSizeTick is unused by the hook — it just forces a re-render after
        // we bump the fake's total size, mirroring react-virtual's onChange.
        ...({ totalSizeTick } as Record<string, never>),
      }),
    { initialProps: { totalSizeTick: 0 } },
  );

  const endPinsBefore = harness.scrollToIndex.mock.calls.filter(
    (c) => c.arguments[1]?.align === "end",
  ).length;

  // Document grows (measured heights settle / more rows stream in).
  act(() => {
    harness.setTotalSize(900);
    rerender({ totalSizeTick: 1 });
  });

  const endPinsAfter = harness.scrollToIndex.mock.calls.filter(
    (c) => c.arguments[1]?.align === "end",
  ).length;
  assert.ok(
    endPinsAfter > endPinsBefore,
    "expected a re-pin to bottom when total size grew while pinned",
  );
  const lastEnd = harness.scrollToIndex.mock.calls
    .filter((c) => c.arguments[1]?.align === "end")
    .at(-1);
  assert.equal(lastEnd?.arguments[0], rows.length - 1);
});

test("first-load settle: does NOT re-pin after the user scrolls away", () => {
  const messages = [message({ id: "a" }), message({ id: "b" })];
  const rows = buildVirtualTimelineRows(messages);
  const harness = makeVirtualizer(100);
  // Container reads as NOT at bottom — the initial syncScrollState/scroll state
  // leaves stickToBottom false, so the settle effect must not yank back down.
  const containerRef = makeContainerRef(false);

  const { result, rerender } = renderHook(
    ({ totalSizeTick }: { totalSizeTick: number }) =>
      useVirtualTimelineScroll({
        channelId: "c1",
        isLoading: false,
        messages,
        rows,
        scrollContainerRef: containerRef,
        virtualizer: harness.virtualizer,
        scrollMarginReady: true,
        ...({ totalSizeTick } as Record<string, never>),
      }),
    { initialProps: { totalSizeTick: 0 } },
  );

  // User has scrolled up — reflect that through syncScrollState.
  act(() => {
    result.current.syncScrollState();
  });
  const endPinsBefore = harness.scrollToIndex.mock.calls.filter(
    (c) => c.arguments[1]?.align === "end",
  ).length;

  act(() => {
    harness.setTotalSize(900);
    rerender({ totalSizeTick: 1 });
  });

  const endPinsAfter = harness.scrollToIndex.mock.calls.filter(
    (c) => c.arguments[1]?.align === "end",
  ).length;
  assert.equal(
    endPinsAfter,
    endPinsBefore,
    "must not re-pin to bottom once the user has scrolled away",
  );
});

test("init pin waits for scrollMarginReady, then pins once the margin lands", () => {
  // The first-load flash (step 5): the init bottom pin and the scrollMargin
  // re-measure are sibling layout effects racing in the same `isLoading→false`
  // commit. If the pin fires while the margin is still the stale pre-mount `0`,
  // it lands `scrollMargin` px short of true bottom and paints rows out of
  // place before re-anchoring. Gate: while `scrollMarginReady` is false, the
  // init pin must NOT fire; once it flips true, it pins exactly once.
  const messages = [message({ id: "a" }), message({ id: "b" })];
  const rows = buildVirtualTimelineRows(messages);
  const { scrollToIndex, virtualizer } = makeVirtualizer();

  const { rerender } = renderHook(
    ({ scrollMarginReady }: { scrollMarginReady: boolean }) =>
      useVirtualTimelineScroll({
        channelId: "c1",
        isLoading: false,
        messages,
        rows,
        scrollContainerRef: makeContainerRef(true),
        virtualizer,
        scrollMarginReady,
      }),
    { initialProps: { scrollMarginReady: false } },
  );

  // Margin not yet measured — the init pin must hold.
  assert.equal(
    scrollToIndex.mock.calls.filter((c) => c.arguments[1]?.align === "end")
      .length,
    0,
    "init pin must not fire before the scroll margin is measured",
  );

  // Margin lands — the init pin fires now, against a trustworthy offset.
  act(() => {
    rerender({ scrollMarginReady: true });
  });

  const endPins = scrollToIndex.mock.calls.filter(
    (c) => c.arguments[1]?.align === "end",
  );
  assert.equal(
    endPins.length,
    1,
    "init pin must fire exactly once after the margin is measured",
  );
  assert.equal(endPins[0]?.arguments[0], rows.length - 1);
});
