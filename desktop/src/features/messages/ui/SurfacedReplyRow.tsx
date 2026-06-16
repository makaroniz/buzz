import type { TimelineMessage } from "@/features/messages/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { surfacedReplyLabel } from "./surfacedReplyLabel.mjs";

/**
 * A read-time pointer that surfaces buried agent replies to the viewer up to
 * the root timeline, at the most-recent reply's `createdAt`. The real messages
 * are never moved or copied; clicking navigates DOWN to the most-recent reply in
 * its thread (open the thread at its `rootId`, scroll to and highlight it).
 *
 * One pointer represents a whole thread's buried replies, framed as
 * "{author} replied to you" with a single-line snippet of the most-recent reply
 * so the pill is self-explanatory at a glance. For `count > 1` a quiet
 * "· N replies" suffix follows the snippet; the count is never the headline. An
 * empty/whitespace-only reply body drops the snippet (no empty quotes).
 *
 * Reuses the visual idiom of `MessageThreadSummaryRow` (rounded pill, avatar,
 * muted label) but is a distinct component: a thread summary means
 * reply-count/participants and opens a panel rooted at itself, whereas this
 * points at the most-recent buried reply and navigates down to it.
 */
export function SurfacedReplyRow({
  count,
  message,
  onNavigate,
}: {
  count: number;
  message: TimelineMessage;
  onNavigate: (message: TimelineMessage) => void;
}) {
  const { snippet, countSuffix } = surfacedReplyLabel({
    body: message.body,
    count,
  });
  return (
    <div className="relative pb-1 pt-0.5">
      <button
        aria-label={`Go to ${message.author}'s reply to you`}
        className="group relative isolate inline-flex h-8 w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-full text-left text-xs font-medium text-muted-foreground transition-[color,opacity] before:pointer-events-none before:absolute before:-bottom-0.5 before:-left-0.5 before:-right-2 before:-top-0.5 before:-z-10 before:rounded-full before:content-[''] before:transition-[background-color,box-shadow] hover:text-foreground hover:opacity-90 hover:before:bg-background/95 hover:before:ring-1 hover:before:ring-border/70 focus-visible:outline-hidden focus-visible:before:bg-background/95 focus-visible:before:ring-1 focus-visible:before:ring-ring"
        data-surfaced-reply-id={message.id}
        data-testid="surfaced-reply-row"
        onClick={() => onNavigate(message)}
        type="button"
      >
        <UserAvatar
          avatarUrl={message.avatarUrl ?? null}
          className="ml-0.5 h-7 w-7"
          displayName={message.author}
          size="sm"
        />
        <div className="flex min-w-0 items-baseline gap-1">
          <span className="shrink-0 font-medium transition-colors group-hover:text-foreground">
            {message.author}
          </span>
          <span className="shrink-0 font-normal text-muted-foreground/70">
            replied to you
          </span>
          {snippet ? (
            <span className="truncate font-normal text-muted-foreground/50">
              — “{snippet}”
            </span>
          ) : null}
          {countSuffix ? (
            <span className="shrink-0 font-normal text-muted-foreground/50">
              · {countSuffix}
            </span>
          ) : null}
        </div>
      </button>
    </div>
  );
}
