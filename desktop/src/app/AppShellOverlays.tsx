import * as React from "react";

import type { ChannelBrowserInitialTab } from "@/features/channels/ui/ChannelBrowserDialog";
import type { ChannelCreateInput } from "@/features/channels/ui/ChannelCreateForm";
import type { Channel } from "@/shared/api/types";

const ChannelBrowserDialog = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelBrowserDialog");
  return { default: module.ChannelBrowserDialog };
});

const ChannelManagementSheet = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelManagementSheet");
  return { default: module.ChannelManagementSheet };
});

export type BrowseDialogType = {
  channelType: "stream" | "forum";
  initialTab: ChannelBrowserInitialTab;
} | null;

type AppShellOverlaysProps = {
  activeChannel: Channel | null;
  browseDialogType: BrowseDialogType;
  channels: Channel[];
  currentPubkey?: string;
  isChannelManagementOpen: boolean;
  isCreatingChannel: boolean;
  isCreatingForum: boolean;
  onBrowseChannelJoin: (channelId: string) => Promise<void>;
  onCreateChannel: (input: ChannelCreateInput) => Promise<void>;
  onCreateForum: (input: ChannelCreateInput) => Promise<void>;
  onBrowseDialogOpenChange: (open: boolean) => void;
  onChannelManagementOpenChange: (open: boolean) => void;
  onDeleteActiveChannel: () => void;
  onSelectChannel: (channelId: string) => void;
};

export function AppShellOverlays({
  activeChannel,
  browseDialogType,
  channels,
  currentPubkey,
  isChannelManagementOpen,
  isCreatingChannel,
  isCreatingForum,
  onBrowseChannelJoin,
  onCreateChannel,
  onCreateForum,
  onBrowseDialogOpenChange,
  onChannelManagementOpenChange,
  onDeleteActiveChannel,
  onSelectChannel,
}: AppShellOverlaysProps) {
  return (
    <>
      {browseDialogType !== null ? (
        <React.Suspense fallback={null}>
          <ChannelBrowserDialog
            channels={channels}
            channelTypeFilter={browseDialogType.channelType}
            initialTab={browseDialogType.initialTab}
            isCreating={
              browseDialogType.channelType === "forum"
                ? isCreatingForum
                : isCreatingChannel
            }
            onCreate={
              browseDialogType.channelType === "forum"
                ? onCreateForum
                : onCreateChannel
            }
            onJoinChannel={onBrowseChannelJoin}
            onOpenChange={onBrowseDialogOpenChange}
            onSelectChannel={onSelectChannel}
            open={true}
          />
        </React.Suspense>
      ) : null}

      {isChannelManagementOpen && activeChannel !== null ? (
        <React.Suspense fallback={null}>
          <ChannelManagementSheet
            channel={activeChannel}
            currentPubkey={currentPubkey}
            onDeleted={onDeleteActiveChannel}
            onOpenChange={onChannelManagementOpenChange}
            open={true}
          />
        </React.Suspense>
      ) : null}
    </>
  );
}
