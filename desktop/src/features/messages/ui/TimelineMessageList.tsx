import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  formatDayHeading,
  isSameDay,
  startOfLocalDaySeconds,
} from "@/features/messages/lib/dateFormatters";
import { collapseSurfacedReplies } from "@/features/messages/lib/collapseSurfacedReplies.mjs";
import { surfaceReplies } from "@/features/messages/lib/surfaceReplies.mjs";
import { buildSurfacedByRoot } from "@/features/messages/lib/surfacedByRoot.mjs";
import {
  getThreadReplyIndentRem,
  THREAD_REPLY_BODY_OFFSET_REM,
  threadReplyLength,
} from "@/features/messages/lib/threadTreeLayout";
import {
  buildMainTimelineEntries,
  shouldRenderUnreadDivider,
} from "@/features/messages/lib/threadPanel";
import {
  buildVideoReviewCommentsForRoot,
  buildVideoReviewContextForMessage,
  hasVideoAttachment,
} from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SurfacedReplyRow } from "./SurfacedReplyRow";
import { SystemMessageRow } from "./SystemMessageRow";
import { UnreadDivider } from "./UnreadDivider";

type TimelineMessageListProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  currentPubkey?: string;
  /** Event id of the oldest unread top-level message; renders a "New" divider above it. */
  firstUnreadMessageId?: string | null;
  followThreadById?: (rootId: string) => void;
  highlightedMessageId?: string | null;
  isFollowingThreadById?: (rootId: string) => boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
  messageFooters?: Record<string, React.ReactNode>;
  mainEntries?: ReturnType<typeof buildMainTimelineEntries>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  isSendingVideoReviewComment?: boolean;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
  /** Per-thread unread counts keyed by thread root id. */
  threadUnreadCounts?: ReadonlyMap<string, number>;
};

type TimelineDayRow = {
  key: string;
  label: string;
  type: "day";
};

type TimelineUnreadRow = {
  key: string;
  type: "unread";
};

type TimelineMessageRowModel = {
  key: string;
  message: TimelineMessage;
  summary: ReturnType<typeof buildMainTimelineEntries>[number]["summary"];
  /** Buried replies-to-viewer in this root's thread, surfaced as an attached pill. */
  surfaced?: { message: TimelineMessage; count: number };
  type: "message";
};

type TimelineRenderRow =
  | TimelineDayRow
  | TimelineUnreadRow
  | TimelineMessageRowModel;

type TimelineNonDayRow = TimelineUnreadRow | TimelineMessageRowModel;

type TimelineDayGroup = {
  key: string;
  label: string;
  rows: TimelineNonDayRow[];
};

function buildTimelineRenderRows({
  agentPubkeys,
  currentPubkey,
  entries,
  firstUnreadMessageId,
  messages,
}: {
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey?: string;
  entries?: ReturnType<typeof buildMainTimelineEntries>;
  firstUnreadMessageId: string | null;
  messages: TimelineMessage[];
}): TimelineRenderRow[] {
  // The timeline renders only root entries. `isHuman` resolves unknown/undefined
  // pubkeys to human so unrecognized authors under-surface (the fail-safe
  // matching the pure-core contract); `surfaceReplies` keeps only replies
  // p-tagging the viewer, then `collapseSurfacedReplies` folds each thread's
  // buried replies into one representative carrying the group count. Each
  // collapsed entry is ATTACHED to its thread-root entry (keyed by the root's
  // own id) and renders as a pill below that root — never as a standalone row.
  // Driving the attach from the entry side means a surfaced thread whose root
  // is not a rendered entry (off-window, or a broadcast-rooted subthread whose
  // root marker points elsewhere) simply contributes no pill: no orphan.
  entries ??= buildMainTimelineEntries(messages);
  const isHuman = (pubkey?: string) =>
    pubkey == null || !agentPubkeys?.has(pubkey);
  const surfacedByRoot = buildSurfacedByRoot(
    collapseSurfacedReplies(surfaceReplies(messages, isHuman, currentPubkey)),
  );
  const rows: TimelineRenderRow[] = [];
  let previousMessage: TimelineMessage | null = null;

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    const message = entry.message;
    const messageRenderKey = message.renderKey ?? message.id;

    if (
      !previousMessage ||
      !isSameDay(previousMessage.createdAt, message.createdAt)
    ) {
      rows.push({
        key: `day-${startOfLocalDaySeconds(message.createdAt)}`,
        label: formatDayHeading(message.createdAt),
        type: "day",
      });
    }
    previousMessage = message;

    // The unread "New" divider only marks a read/unread boundary when there is
    // a message above the first unread. When the first unread is the first
    // rendered top-level entry (fresh/never-read channel), there is nothing
    // above to separate from, so it is suppressed.
    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      rows.push({ key: `unread:${messageRenderKey}`, type: "unread" });
    }

    rows.push({
      key: `msg:${messageRenderKey}`,
      message,
      summary: entry.summary,
      surfaced: surfacedByRoot.get(message.id),
      type: "message",
    });
  }

  return rows;
}

function buildTimelineDayGroups(rows: TimelineRenderRow[]): TimelineDayGroup[] {
  const groups: TimelineDayGroup[] = [];
  let currentGroup: TimelineDayGroup | null = null;

  for (const row of rows) {
    if (row.type === "day") {
      currentGroup = {
        key: row.key,
        label: row.label,
        rows: [],
      };
      groups.push(currentGroup);
      continue;
    }

    if (!currentGroup) {
      currentGroup = {
        key: "day-undated",
        label: "",
        rows: [],
      };
      groups.push(currentGroup);
    }

    currentGroup.rows.push(row);
  }

  return groups;
}

export const TimelineMessageList = React.memo(function TimelineMessageList({
  agentPubkeys,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  firstUnreadMessageId = null,
  followThreadById,
  highlightedMessageId = null,
  isFollowingThreadById,
  isMessageUnreadById,
  messageFooters,
  mainEntries,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
  onReply,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  threadUnreadCounts,
  unfollowThreadById,
}: TimelineMessageListProps) {
  const { goChannel } = useAppNavigation();
  const rows = React.useMemo(
    () =>
      buildTimelineRenderRows({
        agentPubkeys,
        currentPubkey,
        entries: mainEntries,
        firstUnreadMessageId,
        messages,
      }),
    [agentPubkeys, currentPubkey, firstUnreadMessageId, mainEntries, messages],
  );
  const dayGroups = React.useMemo(() => buildTimelineDayGroups(rows), [rows]);

  return (
    <div className="flex flex-col gap-0">
      {dayGroups.map((group) => (
        <section
          className="relative flex flex-col gap-2 before:absolute before:inset-x-0 before:top-4 before:h-px before:bg-border/35 before:content-['']"
          data-day-label={group.label}
          data-testid="message-timeline-day-group"
          key={group.key}
        >
          {group.label ? <DayDivider label={group.label} /> : null}
          {group.rows.map((row) => (
            <TimelineRenderRowView
              agentPubkeys={agentPubkeys}
              allMessages={
                row.type === "message" && hasVideoAttachment(row.message)
                  ? messages
                  : undefined
              }
              channelId={channelId}
              channelName={channelName}
              channelType={channelType}
              currentPubkey={currentPubkey}
              followThreadById={followThreadById}
              goChannel={goChannel}
              highlightedMessageId={highlightedMessageId}
              isFollowingThreadById={isFollowingThreadById}
              isMessageUnreadById={isMessageUnreadById}
              isSendingVideoReviewComment={isSendingVideoReviewComment}
              key={row.key}
              messageFooters={messageFooters}
              onDelete={onDelete}
              onEdit={onEdit}
              onMarkUnread={onMarkUnread}
              onMarkRead={onMarkRead}
              onReply={onReply}
              onSendVideoReviewComment={onSendVideoReviewComment}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
              row={row}
              searchActiveMessageId={searchActiveMessageId}
              searchMatchingMessageIds={searchMatchingMessageIds}
              searchQuery={searchQuery}
              threadUnreadCounts={threadUnreadCounts}
              unfollowThreadById={unfollowThreadById}
            />
          ))}
        </section>
      ))}
    </div>
  );
});

type TimelineRenderRowViewProps = Omit<
  TimelineMessageListProps,
  "firstUnreadMessageId" | "messages" | "personaLookup"
> & {
  allMessages?: TimelineMessage[];
  goChannel: ReturnType<typeof useAppNavigation>["goChannel"];
  row: TimelineRenderRow;
};

const TimelineRenderRowView = React.memo(function TimelineRenderRowView({
  agentPubkeys,
  allMessages,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  followThreadById,
  goChannel,
  highlightedMessageId = null,
  isFollowingThreadById,
  isMessageUnreadById,
  isSendingVideoReviewComment = false,
  messageFooters,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
  onReply,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  row,
  threadUnreadCounts,
  unfollowThreadById,
}: TimelineRenderRowViewProps) {
  const messageForContext = row.type === "message" ? row.message : null;
  const videoReviewContext = React.useMemo(() => {
    if (!allMessages || !messageForContext) {
      return undefined;
    }

    return buildVideoReviewContextForMessage({
      channelId,
      channelName,
      channelType,
      comments: buildVideoReviewCommentsForRoot(
        allMessages,
        messageForContext.id,
      ),
      isSendingVideoReviewComment,
      message: messageForContext,
      onSendVideoReviewComment,
      onToggleReaction,
      profiles,
    });
  }, [
    allMessages,
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messageForContext,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
  ]);

  if (row.type === "day") {
    return <DayDivider label={row.label} />;
  }

  if (row.type === "unread") {
    return <UnreadDivider />;
  }

  const { message, summary } = row;

  // The buried-replies-to-viewer pill, attached below this root message. It is
  // indented to the root's body gutter with the SAME source the thread-summary
  // row uses (`getThreadReplyIndentRem(depth) + THREAD_REPLY_BODY_OFFSET_REM`),
  // so the two pills share one indent definition and stay aligned under the
  // message body — never drifting to the channel margin. Extracted once so all
  // three root-render branches share it. Click navigates DOWN to the most-recent
  // surfaced reply; `threadRootId` prefetches the thread root in
  // ChannelRouteScreen so the open finds the head even when the root is outside
  // the loaded window — the same mechanism search hits use.
  const surfacedPill = row.surfaced ? (
    <div
      style={{
        marginLeft: threadReplyLength(
          getThreadReplyIndentRem(message.depth) + THREAD_REPLY_BODY_OFFSET_REM,
        ),
      }}
    >
      <SurfacedReplyRow
        count={row.surfaced.count}
        message={row.surfaced.message}
        onNavigate={(target) => {
          if (!channelId) return;
          void goChannel(channelId, {
            messageId: target.id,
            threadRootId: target.rootId,
          });
        }}
      />
    </div>
  ) : null;

  if (message.kind === KIND_SYSTEM_MESSAGE) {
    const footer = messageFooters?.[message.id] ?? null;
    return (
      <div className="flex flex-col gap-1">
        <SystemMessageRow
          message={message}
          currentPubkey={currentPubkey}
          onToggleReaction={onToggleReaction}
          profiles={profiles}
        />
        {surfacedPill}
        {footer}
      </div>
    );
  }

  if (summary && onReply) {
    const footer = messageFooters?.[message.id] ?? null;
    const isHighlighted = message.id === highlightedMessageId;
    return (
      <div
        className={cn(
          "group/message relative mx-1 flex flex-col gap-0 rounded-2xl px-0 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
          isHighlighted &&
            "-mx-4 px-4 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-6",
        )}
      >
        <MessageRow
          agentPubkeys={agentPubkeys}
          channelId={channelId}
          highlighted={false}
          hoverBackground={false}
          isFollowingThread={
            isFollowingThreadById
              ? isFollowingThreadById(message.id)
              : undefined
          }
          isUnread={isMessageUnreadById?.(message.id)}
          message={message}
          onDelete={
            onDelete && currentPubkey && message.pubkey === currentPubkey
              ? onDelete
              : undefined
          }
          onEdit={
            onEdit && currentPubkey && message.pubkey === currentPubkey
              ? onEdit
              : undefined
          }
          onFollowThread={
            followThreadById ? () => followThreadById(message.id) : undefined
          }
          onMarkUnread={onMarkUnread}
          onMarkRead={onMarkRead}
          onToggleReaction={onToggleReaction}
          onReply={onReply}
          onUnfollowThread={
            unfollowThreadById
              ? () => unfollowThreadById(message.id)
              : undefined
          }
          profiles={profiles}
          showDepthGuides={false}
          videoReviewContext={videoReviewContext}
        />
        <MessageThreadSummaryRow
          depth={message.depth}
          message={message}
          onOpenThread={onReply}
          showDepthGuides={false}
          summary={summary}
          unreadCount={threadUnreadCounts?.get(message.id)}
        />
        {surfacedPill}
        {footer}
      </div>
    );
  }

  const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
  const isSearchActive = message.id === searchActiveMessageId;
  const footer = messageFooters?.[message.id] ?? null;

  return (
    <div className="flex flex-col gap-1">
      <MessageRow
        agentPubkeys={agentPubkeys}
        channelId={channelId}
        highlighted={message.id === highlightedMessageId || isSearchActive}
        isUnread={isMessageUnreadById?.(message.id)}
        message={message}
        onDelete={
          onDelete && currentPubkey && message.pubkey === currentPubkey
            ? onDelete
            : undefined
        }
        onEdit={
          onEdit && currentPubkey && message.pubkey === currentPubkey
            ? onEdit
            : undefined
        }
        onMarkUnread={onMarkUnread}
        onMarkRead={onMarkRead}
        onToggleReaction={onToggleReaction}
        onReply={onReply}
        profiles={profiles}
        searchQuery={isSearchMatch ? searchQuery : undefined}
        showDepthGuides={false}
        videoReviewContext={videoReviewContext}
      />
      {surfacedPill}
      {footer}
    </div>
  );
});
