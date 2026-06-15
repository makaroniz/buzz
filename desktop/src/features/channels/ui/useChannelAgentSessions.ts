import * as React from "react";

import { useAgentOwnershipQuery } from "@/features/agents/hooks/useCanViewAgentActivity";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel, ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import {
  type ChannelAgentSessionAgent,
  getChannelAgentSessionAgents,
  resolveOpenAgentSessionAgent,
} from "../lib/agentSessionCandidates";

export type { ChannelAgentSessionAgent } from "../lib/agentSessionCandidates";
export {
  buildChannelAgentSessionCandidates,
  getChannelAgentSessionAgents,
  resolveOpenAgentSessionAgent,
} from "../lib/agentSessionCandidates";

type UseChannelAgentSessionsOptions = {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  agentCandidates: ChannelAgentSessionAgent[];
  channelMembers?: ChannelMember[];
  handleOpenThread: (message: TimelineMessage) => void;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
};

export function useChannelAgentSessions({
  activeChannel,
  activeChannelId,
  agentCandidates,
  channelMembers,
  handleOpenThread,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelAgentSessionsOptions) {
  const [openAgentSessionPubkey, setOpenAgentSessionPubkey] = React.useState<
    string | null
  >(null);

  const channelAgentSessionAgents = React.useMemo(
    () =>
      getChannelAgentSessionAgents({
        activeChannel,
        activeChannelId,
        agents: agentCandidates,
        channelMembers,
      }),
    [activeChannel, activeChannelId, agentCandidates, channelMembers],
  );

  const ownershipQuery = useAgentOwnershipQuery(openAgentSessionPubkey);

  const openAgentSessionAgent = React.useMemo(
    () =>
      resolveOpenAgentSessionAgent({
        allAgentCandidates: agentCandidates,
        channelAgentSessionAgents,
        openAgentSessionPubkey,
      }),
    [agentCandidates, channelAgentSessionAgents, openAgentSessionPubkey],
  );

  const closeAgentSession = React.useCallback(() => {
    setOpenAgentSessionPubkey(null);
  }, []);

  const openAgentSession = React.useCallback(
    (pubkey: string) => {
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      setProfilePanelPubkey(null);
      setOpenAgentSessionPubkey(pubkey);
    },
    [
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const selectAgentSession = React.useCallback((pubkey: string) => {
    setOpenAgentSessionPubkey(pubkey);
  }, []);

  const openThreadAndCloseAgentSession = React.useCallback(
    (message: TimelineMessage) => {
      setOpenAgentSessionPubkey(null);
      setProfilePanelPubkey(null);
      handleOpenThread(message);
    },
    [handleOpenThread, setProfilePanelPubkey],
  );

  React.useEffect(() => {
    if (!openAgentSessionPubkey) {
      return;
    }

    const inChannelList = channelAgentSessionAgents.some(
      (agent) =>
        normalizePubkey(agent.pubkey) ===
        normalizePubkey(openAgentSessionPubkey),
    );
    if (inChannelList) {
      return;
    }

    if (ownershipQuery.isLoading || ownershipQuery.data === undefined) {
      return;
    }

    if (!ownershipQuery.data.isOwner) {
      setOpenAgentSessionPubkey(null);
    }
  }, [
    channelAgentSessionAgents,
    openAgentSessionPubkey,
    ownershipQuery.data,
    ownershipQuery.isLoading,
  ]);

  return {
    channelAgentSessionAgents,
    closeAgentSession,
    openAgentSession,
    openAgentSessionAgent,
    openAgentSessionPubkey,
    openThreadAndCloseAgentSession,
    selectAgentSession,
  };
}
