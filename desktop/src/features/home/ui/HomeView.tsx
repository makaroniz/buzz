import * as React from "react";
import {
  Activity,
  AtSign,
  Bot,
  CircleAlert,
  RefreshCcw,
  Search,
} from "lucide-react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useHomeBackgroundSettings } from "@/features/home/useHomeBackgroundSettings";
import { useFeedItemState } from "@/features/home/useFeedItemState";
import { HomeBackgroundLayer } from "@/features/home/ui/HomeBackgroundLayer";
import { HomeSearchPanel } from "@/features/home/ui/HomeSearchPanel";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useContactListQuery, useTimelineQuery } from "@/features/pulse/hooks";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import type {
  Channel,
  FeedItem,
  HomeFeedResponse,
  SearchHit,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";

const FeedSection = React.lazy(async () => {
  const module = await import("./FeedSection");
  return { default: module.FeedSection };
});

const RecentNotesSection = React.lazy(async () => {
  const module = await import("./RecentNotesSection");
  return { default: module.RecentNotesSection };
});

type HomeTab = "search" | "inbox" | "feed";

const tabTriggerClassName =
  "h-8 rounded-full px-4 text-xs font-medium shadow-none transition-colors data-[state=active]:bg-primary data-[state=active]:text-primary-foreground data-[state=active]:shadow-sm";

function HomeLoadingState() {
  return (
    <div className="px-4 py-5 sm:px-6">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
        <div className="grid gap-4">
          {["mentions", "actions"].map((section) => (
            <div key={section}>
              <Skeleton className="mb-2 h-4 w-24" />
              <div className="space-y-0 rounded-md border border-border/60">
                {["a", "b", "c"].map((row) => (
                  <Skeleton className="h-16" key={row} />
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function FeedLoadingState() {
  return (
    <div className="mx-auto w-full max-w-3xl space-y-3 px-4 py-5 sm:px-6">
      {["first", "second", "third"].map((row) => (
        <div
          className="flex items-start gap-2.5 rounded-md border border-border/60 px-3 py-2.5"
          key={row}
        >
          <Skeleton className="h-8 w-8 shrink-0 rounded-full" />
          <div className="min-w-0 flex-1 space-y-2">
            <Skeleton className="h-3 w-28" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-2/3" />
          </div>
        </div>
      ))}
    </div>
  );
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

type HomeViewProps = {
  feed?: HomeFeedResponse;
  isLoading?: boolean;
  errorMessage?: string;
  channels: Channel[];
  currentPubkey?: string;
  availableChannelIds: ReadonlySet<string>;
  onOpenFeedItem: (item: FeedItem) => void;
  onOpenPulse: () => void;
  onOpenSearchResult: (hit: SearchHit) => void;
  onRefresh: () => void;
};

export function HomeView({
  feed,
  isLoading = false,
  errorMessage,
  channels,
  currentPubkey,
  availableChannelIds,
  onOpenFeedItem,
  onOpenPulse,
  onOpenSearchResult,
  onRefresh,
}: HomeViewProps) {
  const [activeTab, setActiveTab] = React.useState<HomeTab>("search");
  const { settings: homeBackgroundSettings } = useHomeBackgroundSettings();
  const { doneSet, markDone, undoDone } = useFeedItemState(currentPubkey);

  // Defer Pulse feed queries until the shell is interactive and the tab is open.
  const startupReady = useDeferredStartup();
  const shouldLoadFollowingFeed = startupReady && activeTab === "feed";
  const deferredPubkey = shouldLoadFollowingFeed ? currentPubkey : undefined;

  const contactListQuery = useContactListQuery(deferredPubkey);
  const contactPubkeys = React.useMemo(
    () => (contactListQuery.data?.contacts ?? []).map((c) => c.pubkey),
    [contactListQuery.data],
  );
  const notesPubkeys = React.useMemo(
    () => [...new Set(contactPubkeys)],
    [contactPubkeys],
  );
  const notesTimelineQuery = useTimelineQuery(
    notesPubkeys,
    shouldLoadFollowingFeed && notesPubkeys.length > 0,
  );
  const recentNotes = notesTimelineQuery.data?.notes ?? [];
  const noteAuthorPubkeys = React.useMemo(
    () => [...new Set(recentNotes.map((n) => n.pubkey))],
    [recentNotes],
  );
  const noteProfilesQuery = useUsersBatchQuery(noteAuthorPubkeys, {
    enabled: noteAuthorPubkeys.length > 0,
  });
  const noteProfiles = noteProfilesQuery.data?.profiles ?? {};
  const relayAgentsQuery = useRelayAgentsQuery({
    enabled: shouldLoadFollowingFeed,
  });
  const agentPubkeySet = React.useMemo(
    () => new Set((relayAgentsQuery.data ?? []).map((a) => a.pubkey)),
    [relayAgentsQuery.data],
  );

  const feedItems = feed
    ? [
        ...feed.feed.mentions,
        ...feed.feed.needsAction,
        ...(feed.feed.activity ?? []),
        ...(feed.feed.agentActivity ?? []),
      ]
    : [];
  const feedProfilesQuery = useUsersBatchQuery(
    feedItems.map((item) => item.pubkey),
    {
      enabled: feedItems.length > 0,
    },
  );
  const feedProfiles = feedProfilesQuery.data?.profiles;

  function renderInboxPanel() {
    if (isLoading && !feed) {
      return <HomeLoadingState />;
    }

    if (!feed) {
      return (
        <div className="px-4 py-5 sm:px-6">
          <div className="mx-auto flex w-full max-w-3xl flex-col gap-4">
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-4 py-5">
              <p className="text-base font-semibold tracking-tight">
                Home feed unavailable
              </p>
              <p className="mt-2 text-sm text-muted-foreground">
                {errorMessage ?? "The relay did not return a feed response."}
              </p>
              <Button className="mt-5" onClick={onRefresh} type="button">
                <RefreshCcw className="h-4 w-4" />
                Try again
              </Button>
            </div>
          </div>
        </div>
      );
    }

    const priorityCount =
      feed.feed.needsAction.length + feed.feed.mentions.length;

    return (
      <div className="px-4 py-5 sm:px-6">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
          <React.Suspense fallback={null}>
            <div className="grid gap-6">
              <section className="rounded-2xl border border-primary/15 bg-background/35 p-4 shadow-[0_4px_24px_rgba(0,0,0,0.08)] backdrop-blur-xl supports-[backdrop-filter]:bg-background/25 dark:shadow-[0_4px_24px_rgba(0,0,0,0.35)]">
                <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <div className="flex items-center gap-2">
                      <CircleAlert className="h-4 w-4 text-primary" />
                      <h2 className="text-sm font-semibold tracking-tight">
                        Priority inbox
                      </h2>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      Mentions and items waiting for you are grouped here first.
                    </p>
                  </div>
                  <span className="rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary">
                    {priorityCount} open
                  </span>
                </div>

                <div className="grid gap-5">
                  <FeedSection
                    availableChannelIds={availableChannelIds}
                    currentPubkey={currentPubkey}
                    profiles={feedProfiles}
                    doneSet={doneSet}
                    emptyDescription="Approval requests and reminders will appear here."
                    emptyTitle="Nothing needs action"
                    icon={CircleAlert}
                    items={feed.feed.needsAction}
                    onMarkDone={markDone}
                    onOpenItem={onOpenFeedItem}
                    onUndoDone={undoDone}
                    showDoneAction={true}
                    title="Needs Action"
                  />
                  <FeedSection
                    availableChannelIds={availableChannelIds}
                    currentPubkey={currentPubkey}
                    profiles={feedProfiles}
                    doneSet={doneSet}
                    emptyDescription="When someone mentions you, it will land here."
                    emptyTitle="No mentions right now"
                    icon={AtSign}
                    items={feed.feed.mentions}
                    onMarkDone={markDone}
                    onOpenItem={onOpenFeedItem}
                    onUndoDone={undoDone}
                    showDoneAction={false}
                    title="Mentions"
                  />
                </div>
              </section>

              <section className="rounded-2xl border border-border/50 bg-background/30 p-4 shadow-[0_4px_24px_rgba(0,0,0,0.08)] backdrop-blur-xl supports-[backdrop-filter]:bg-background/20 dark:shadow-[0_4px_24px_rgba(0,0,0,0.35)]">
                <div className="mb-4">
                  <h2 className="text-sm font-semibold tracking-tight">
                    Everything else
                  </h2>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Channel activity and agent updates that are useful, but not
                    urgent.
                  </p>
                </div>

                <div className="grid gap-5">
                  <FeedSection
                    availableChannelIds={availableChannelIds}
                    currentPubkey={currentPubkey}
                    profiles={feedProfiles}
                    doneSet={doneSet}
                    emptyDescription="Recent channel messages and forum posts will show up here."
                    emptyTitle="No channel activity yet"
                    icon={Activity}
                    items={feed.feed.activity ?? []}
                    onMarkDone={markDone}
                    onOpenItem={onOpenFeedItem}
                    onUndoDone={undoDone}
                    showDoneAction={false}
                    title="Channel Activity"
                  />
                  <FeedSection
                    availableChannelIds={availableChannelIds}
                    currentPubkey={currentPubkey}
                    profiles={feedProfiles}
                    doneSet={doneSet}
                    emptyDescription="Agent job requests, progress, and results will appear here."
                    emptyTitle="No agent updates yet"
                    icon={Bot}
                    items={feed.feed.agentActivity ?? []}
                    onMarkDone={markDone}
                    onOpenItem={onOpenFeedItem}
                    onUndoDone={undoDone}
                    showDoneAction={false}
                    title="Agent Updates"
                  />
                </div>
              </section>
            </div>
          </React.Suspense>
        </div>
      </div>
    );
  }

  function renderFollowingFeedPanel() {
    if (!currentPubkey) {
      return (
        <EmptyPanel
          description="Choose a workspace identity before loading followed people and agents."
          title="Feed unavailable"
        />
      );
    }

    if (!startupReady) {
      return <FeedLoadingState />;
    }

    if (contactListQuery.isLoading || notesTimelineQuery.isLoading) {
      return <FeedLoadingState />;
    }

    if (contactPubkeys.length === 0) {
      return (
        <EmptyPanel
          description="Follow people and agents in Pulse to build this feed."
          title="Nothing in your feed yet"
        />
      );
    }

    if (recentNotes.length === 0) {
      return (
        <EmptyPanel
          description="Followed people and agents have not posted notes yet."
          title="No feed updates yet"
        />
      );
    }

    return (
      <div className="px-4 py-5 sm:px-6">
        <div className="mx-auto w-full max-w-3xl">
          <React.Suspense fallback={<FeedLoadingState />}>
            <RecentNotesSection
              agentPubkeys={agentPubkeySet}
              notes={recentNotes}
              onOpenPulse={onOpenPulse}
              profiles={noteProfiles}
              title="Following Feed"
            />
          </React.Suspense>
        </div>
      </div>
    );
  }

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => setActiveTab(value as HomeTab)}
      className="relative isolate flex min-h-0 flex-1 flex-col overflow-hidden"
    >
      <HomeBackgroundLayer className="z-0" settings={homeBackgroundSettings} />

      <div className="relative z-10 flex justify-center px-4 pt-14 sm:px-6">
        <TabsList className="h-10 gap-1 rounded-full border border-border/60 bg-card/80 p-1 shadow-sm backdrop-blur">
          <TabsTrigger
            aria-label="Search"
            className="h-8 w-10 rounded-full px-0 data-[state=active]:bg-primary data-[state=active]:text-primary-foreground"
            value="search"
          >
            <Search className="h-4 w-4" />
          </TabsTrigger>
          <TabsTrigger className={tabTriggerClassName} value="inbox">
            Inbox
          </TabsTrigger>
          <TabsTrigger className={tabTriggerClassName} value="feed">
            Feed
          </TabsTrigger>
        </TabsList>
      </div>

      <TabsContent
        className="relative z-10 mt-0 min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain"
        value="search"
      >
        <HomeSearchPanel
          channels={channels}
          currentPubkey={currentPubkey}
          onOpenSearchResult={onOpenSearchResult}
        />
      </TabsContent>

      <TabsContent
        className="relative z-10 mt-0 min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain"
        value="inbox"
      >
        {renderInboxPanel()}
      </TabsContent>

      <TabsContent
        className="relative z-10 mt-0 min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain"
        value="feed"
      >
        {renderFollowingFeedPanel()}
      </TabsContent>
    </Tabs>
  );
}
