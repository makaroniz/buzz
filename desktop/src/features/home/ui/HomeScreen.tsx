import type { Channel, FeedItem, SearchHit } from "@/shared/api/types";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";

type HomeScreenProps = {
  channels: Channel[];
  availableChannelIds: ReadonlySet<string>;
  currentPubkey?: string;
  onOpenFeedItem: (item: FeedItem) => void;
  onOpenPulse: () => void;
  onOpenSearchResult: (hit: SearchHit) => void;
};

export function HomeScreen({
  channels,
  availableChannelIds,
  currentPubkey,
  onOpenFeedItem,
  onOpenPulse,
  onOpenSearchResult,
}: HomeScreenProps) {
  const homeFeedQuery = useHomeFeedQuery();

  return (
    <>
      <ChatHeader
        description="Personalized activity feed for mentions, reminders, channel activity, and agent work."
        mode="home"
        overlaysContent
        title="Home"
      />

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <HomeView
          availableChannelIds={availableChannelIds}
          channels={channels}
          currentPubkey={currentPubkey}
          errorMessage={
            homeFeedQuery.error instanceof Error
              ? homeFeedQuery.error.message
              : undefined
          }
          feed={homeFeedQuery.data}
          isLoading={homeFeedQuery.isLoading}
          onOpenFeedItem={onOpenFeedItem}
          onOpenPulse={onOpenPulse}
          onOpenSearchResult={onOpenSearchResult}
          onRefresh={() => {
            void homeFeedQuery.refetch();
          }}
        />
      </div>
    </>
  );
}
