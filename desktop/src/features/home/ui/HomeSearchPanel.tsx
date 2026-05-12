import * as React from "react";
import { ArrowRight, FileText, Hash, Search } from "lucide-react";

import {
  resolveUserLabel,
  resolveUserSecondaryLabel,
} from "@/features/profile/lib/identity";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { Channel, SearchHit } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MIN_QUERY_LENGTH = 2;

function describeSearchHit(hit: SearchHit) {
  switch (hit.kind) {
    case 1:
      return "Note";
    case 45001:
      return "Forum post";
    case 45003:
      return "Forum reply";
    case 43001:
      return "Agent job";
    case 43003:
      return "Agent update";
    case 46010:
      return "Approval request";
    default:
      return "Message";
  }
}

function truncateContent(content: string) {
  const trimmed = content.trim();
  if (trimmed.length === 0) {
    return "No message body.";
  }

  return trimmed.length <= 180 ? trimmed : `${trimmed.slice(0, 177)}...`;
}

function formatRelativeTime(unixSeconds: number) {
  const diff = Math.floor(Date.now() / 1_000) - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 60 * 60) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 60 * 60 * 24) return `${Math.floor(diff / (60 * 60))}h ago`;

  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1_000));
}

function EmptyPanel({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col items-center justify-center px-4 py-16 text-center sm:px-6">
      <p className="text-sm font-medium text-muted-foreground">{title}</p>
      <p className="mt-1 max-w-md text-xs text-muted-foreground/70">
        {description}
      </p>
    </div>
  );
}

type HomeSearchPanelProps = {
  channels: Channel[];
  currentPubkey?: string;
  onOpenSearchResult: (hit: SearchHit) => void;
};

export function HomeSearchPanel({
  channels,
  currentPubkey,
  onOpenSearchResult,
}: HomeSearchPanelProps) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [debouncedSearchQuery, setDebouncedSearchQuery] = React.useState("");
  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  React.useEffect(() => {
    const trimmed = searchQuery.trim();
    if (trimmed.length < MIN_QUERY_LENGTH) {
      setDebouncedSearchQuery("");
      return;
    }

    const timeout = window.setTimeout(
      () => setDebouncedSearchQuery(trimmed),
      300,
    );
    return () => window.clearTimeout(timeout);
  }, [searchQuery]);

  const searchMessagesQuery = useSearchMessagesQuery(debouncedSearchQuery, {
    enabled: true,
    limit: 10,
  });
  const searchResults = searchMessagesQuery.data?.hits ?? [];
  const searchProfilesQuery = useUsersBatchQuery(
    searchResults.map((hit) => hit.pubkey),
    { enabled: searchResults.length > 0 },
  );
  const searchProfiles = searchProfilesQuery.data?.profiles;

  return (
    <div
      className={`mx-auto flex min-h-full w-full max-w-3xl flex-col px-4 py-8 sm:px-6 ${
        debouncedSearchQuery.length < MIN_QUERY_LENGTH
          ? "justify-center"
          : "justify-start"
      }`}
    >
      <div className="flex items-center gap-3 rounded-full border border-border/50 bg-background/70 px-5 py-4 shadow-[0_4px_24px_rgba(0,0,0,0.08)] backdrop-blur-xl supports-[backdrop-filter]:bg-background/55 dark:shadow-[0_4px_24px_rgba(0,0,0,0.35)]">
        <Search className="h-5 w-5 text-muted-foreground" />
        <Input
          autoFocus
          className="h-auto border-0 bg-transparent px-0 py-0 text-base shadow-none focus-visible:ring-0"
          onChange={(event) => setSearchQuery(event.target.value)}
          placeholder="Search messages, approvals, and forum posts"
          value={searchQuery}
        />
      </div>

      {debouncedSearchQuery.length < MIN_QUERY_LENGTH ? (
        <p className="mt-3 text-center text-xs text-muted-foreground">
          Type at least two characters to search accessible conversations.
        </p>
      ) : searchMessagesQuery.isLoading ? (
        <div className="mt-5 space-y-3">
          {["first", "second", "third"].map((row) => (
            <div
              className="rounded-2xl border border-border/80 bg-card/60 p-4"
              key={row}
            >
              <Skeleton className="h-4 w-32" />
              <Skeleton className="mt-3 h-4 w-full" />
              <Skeleton className="mt-2 h-4 w-3/4" />
            </div>
          ))}
        </div>
      ) : searchMessagesQuery.error instanceof Error ? (
        <EmptyPanel
          description={searchMessagesQuery.error.message}
          title="Search unavailable"
        />
      ) : searchResults.length === 0 ? (
        <EmptyPanel
          description="Try a different keyword, channel name, or phrase from the message body."
          title="No matches found"
        />
      ) : (
        <div className="mt-5 space-y-2" data-testid="home-search-results">
          <div className="flex items-center justify-between px-2 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            <span>
              {searchMessagesQuery.data?.found ?? searchResults.length} results
            </span>
            <span>Select to open</span>
          </div>

          {searchResults.map((hit) => {
            const channel = hit.channelId
              ? channelLookup.get(hit.channelId)
              : undefined;
            const authorLabel = resolveUserLabel({
              pubkey: hit.pubkey,
              currentPubkey,
              profiles: searchProfiles,
              preferResolvedSelfLabel: true,
            });
            const authorSecondaryLabel = resolveUserSecondaryLabel({
              pubkey: hit.pubkey,
              profiles: searchProfiles,
            });

            return (
              <button
                className="w-full rounded-2xl border border-border/80 bg-card/60 px-4 py-4 text-left shadow-sm outline-none transition-colors hover:border-primary/20 hover:bg-accent"
                data-testid={`home-search-result-${hit.eventId}`}
                key={hit.eventId}
                onClick={() => onOpenSearchResult(hit)}
                type="button"
              >
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-2xl bg-secondary text-secondary-foreground">
                    {channel?.channelType === "forum" ? (
                      <FileText className="h-4 w-4" />
                    ) : (
                      <Hash className="h-4 w-4" />
                    )}
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <p className="text-sm font-semibold tracking-tight">
                        {hit.channelName}
                      </p>
                      <Badge variant="secondary">
                        {describeSearchHit(hit)}
                      </Badge>
                      <span className="flex items-center gap-1 text-xs text-muted-foreground">
                        <UserAvatar
                          avatarUrl={
                            searchProfiles?.[hit.pubkey.toLowerCase()]
                              ?.avatarUrl ?? null
                          }
                          displayName={authorLabel}
                          size="xs"
                        />
                        {authorLabel}
                      </span>
                      <p className="ml-auto whitespace-nowrap text-xs text-muted-foreground">
                        {formatRelativeTime(hit.createdAt)}
                      </p>
                    </div>
                    {authorSecondaryLabel ? (
                      <p className="mt-1 text-xs text-muted-foreground">
                        {authorSecondaryLabel}
                      </p>
                    ) : null}
                    <p className="mt-2 text-sm leading-6 text-foreground">
                      {truncateContent(hit.content)}
                    </p>
                  </div>

                  <ArrowRight className="mt-1 h-4 w-4 shrink-0 text-muted-foreground" />
                </div>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
