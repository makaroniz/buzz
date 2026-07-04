// @ts-nocheck
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { GitPullRequest } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useAppShell } from "@/app/AppShellContext";
import {
  useManagedAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import {
  useArchiveChatMutation,
  useChatMetadataListQuery,
  useChatMetadataQuery,
  useChatsQuery,
  useUpdateChatMetadataMutation,
} from "@/features/chats/hooks";
import { buildChatProjects } from "@/features/chats/lib/chatProjects";
import { isSharedChatMetadata } from "@/features/chats/lib/chatShared";
import { ChatList } from "@/features/chats/ui/ChatList";
import {
  toggleStoredChatPin,
  useStoredChatPins,
} from "@/features/chats/lib/chatPinStorage";
import {
  mergeChatProjects,
  upsertStoredChatProject,
  useStoredChatProjects,
} from "@/features/chats/lib/chatProjectStorage";
import {
  buildChatCanvasContent,
  buildProjectSetupContext,
  uniqueMentionPubkeys,
} from "@/features/chats/lib/chatSetup";
import { ChatDetail } from "@/features/chats/ui/ChatDetail";
import { ChatHeaderActions } from "@/features/chats/ui/ChatHeaderActions";
import { ChatRenameDialog } from "@/features/chats/ui/ChatRenameDialog";
import { QuickStartChat } from "@/features/chats/ui/QuickStartChat";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import { ensureWelcomeGuideAgentInChannel } from "@/features/onboarding/welcomeGuide";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useIdentityQuery } from "@/shared/api/hooks";
import { addChannelMembers, getCanvas, setCanvas } from "@/shared/api/tauri";
import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";

type ChatsScreenProps = {
  initialProjectId?: string | null;
  selectedChatId?: string | null;
};

async function backfillBlankChatCanvas(channelId: string, content: string) {
  const existing = await getCanvas(channelId);
  if (existing.content?.trim()) {
    return false;
  }
  await setCanvas({ channelId, content });
  return true;
}

async function addBotToChat(channelId: string, pubkey: string) {
  const result = await addChannelMembers({
    channelId,
    pubkeys: [pubkey],
    role: "bot",
  });
  const error = result.errors.find(
    (entry) => normalizePubkey(entry.pubkey) === normalizePubkey(pubkey),
  );
  if (error && !error.error.toLowerCase().includes("already")) {
    throw new Error(error.error);
  }
}

export function ChatsScreen({
  initialProjectId,
  selectedChatId = null,
}: ChatsScreenProps) {
  const queryClient = useQueryClient();
  const { activeWorkspace } = useWorkspaces();
  const { goChat, goChats } = useAppNavigation();
  const {
    getChannelReadAt,
    markChannelRead,
    readStateVersion,
    unreadChannelCounts,
    unreadChannelIds,
  } = useAppShell();
  const identityQuery = useIdentityQuery();
  const chatsQuery = useChatsQuery();
  const chats = chatsQuery.data ?? [];
  const metadataListQuery = useChatMetadataListQuery();
  const allMetadata = metadataListQuery.data ?? [];
  const storedChatProjects = useStoredChatProjects(activeWorkspace?.id);
  const templatesQuery = useChannelTemplatesQuery();
  const templates = templatesQuery.data ?? [];
  const identityPubkey = identityQuery.data?.pubkey;
  const ownedMetadata = React.useMemo(
    () =>
      allMetadata.filter(
        (metadata) => !isSharedChatMetadata(metadata, identityPubkey),
      ),
    [allMetadata, identityPubkey],
  );
  const chatProjects = React.useMemo(
    () =>
      mergeChatProjects(storedChatProjects, buildChatProjects(ownedMetadata)),
    [ownedMetadata, storedChatProjects],
  );
  const backfilledCanvasKeysRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    if (
      chatsQuery.isLoading ||
      metadataListQuery.isLoading ||
      templatesQuery.isLoading
    ) {
      return;
    }

    const chatsById = new Map(chats.map((chat) => [chat.id, chat]));
    const templatesById = new Map(
      templates.map((template) => [template.id, template]),
    );

    for (const metadata of ownedMetadata) {
      const projectId = metadata.projectId?.trim();
      const projectName = metadata.projectName?.trim();
      if (!projectId || !projectName) {
        continue;
      }

      const chat = chatsById.get(metadata.channelId);
      if (!chat) {
        continue;
      }

      const templateId =
        metadata.projectTemplateId?.trim() || metadata.templateId?.trim() || "";
      const template = templateId ? templatesById.get(templateId) : null;
      const project = {
        id: projectId,
        name: projectName,
        path: metadata.projectPath?.trim() || null,
        templateId: templateId || null,
        updatedAt: metadata.updatedAt,
        chatCount: 1,
      };
      const leadingContent = buildProjectSetupContext({
        project,
        templateName: template?.name ?? null,
      });
      const content = buildChatCanvasContent({
        channelName: metadata.title?.trim() || chat.name,
        leadingContent,
        template,
      });
      if (!content) {
        continue;
      }

      const backfillKey = [
        chat.id,
        project.id,
        project.path ?? "",
        template?.id ?? "",
        template?.updatedAt ?? "",
        content.length,
      ].join(":");
      if (backfilledCanvasKeysRef.current.has(backfillKey)) {
        continue;
      }
      backfilledCanvasKeysRef.current.add(backfillKey);

      void backfillBlankChatCanvas(chat.id, content)
        .then((didBackfill) => {
          if (didBackfill) {
            void queryClient.invalidateQueries({
              queryKey: ["channel-canvas", chat.id],
            });
          }
        })
        .catch((error) => {
          console.warn("Failed to backfill chat canvas", chat.id, error);
        });
    }
  }, [
    chats,
    chatsQuery.isLoading,
    metadataListQuery.isLoading,
    ownedMetadata,
    queryClient,
    templates,
    templatesQuery.isLoading,
  ]);
  const metadataByChatId = React.useMemo(
    () =>
      new Map(allMetadata.map((metadata) => [metadata.channelId, metadata])),
    [allMetadata],
  );
  const selectedChat =
    selectedChatId !== null
      ? (chats.find((chat) => chat.id === selectedChatId) ?? null)
      : null;

  // Remember the last-viewed chat per workspace so returning to the Chats
  // tab lands there instead of the new-chat screen. Explicit new-chat
  // navigations carry a projectId in the route search (initialProjectId is
  // then non-undefined) and are never redirected.
  const lastChatStorageKey = activeWorkspace?.id
    ? `buzz:chats:last-viewed:${activeWorkspace.id}`
    : null;
  React.useEffect(() => {
    if (!lastChatStorageKey || !selectedChat) {
      return;
    }
    try {
      window.localStorage.setItem(lastChatStorageKey, selectedChat.id);
    } catch {
      // Storage unavailable — restore is best-effort.
    }
  }, [lastChatStorageKey, selectedChat]);
  const attemptedChatRestoreRef = React.useRef(false);
  React.useEffect(() => {
    if (selectedChatId !== null) {
      // Viewing a chat re-arms the restore for the next plain visit.
      attemptedChatRestoreRef.current = false;
      return;
    }
    if (
      initialProjectId !== undefined ||
      chatsQuery.isLoading ||
      attemptedChatRestoreRef.current
    ) {
      return;
    }
    attemptedChatRestoreRef.current = true;
    let storedChatId: string | null = null;
    try {
      storedChatId = lastChatStorageKey
        ? window.localStorage.getItem(lastChatStorageKey)
        : null;
    } catch {
      storedChatId = null;
    }
    if (storedChatId && chats.some((chat) => chat.id === storedChatId)) {
      void goChat(storedChatId, { replace: true });
    }
  }, [
    chats,
    chatsQuery.isLoading,
    goChat,
    initialProjectId,
    lastChatStorageKey,
    selectedChatId,
  ]);

  const metadataQuery = useChatMetadataQuery(selectedChat?.id);
  const metadata = metadataQuery.data ?? null;
  const managedAgentsQuery = useManagedAgentsQuery();
  const metadataDefaultAgentPubkey = metadata?.defaultAgentPubkey ?? null;
  const defaultAgent = React.useMemo(() => {
    if (!metadataDefaultAgentPubkey) {
      return null;
    }
    const normalizedDefaultAgentPubkey = normalizePubkey(
      metadataDefaultAgentPubkey,
    );
    return (
      (managedAgentsQuery.data ?? []).find(
        (agent) =>
          normalizePubkey(agent.pubkey) === normalizedDefaultAgentPubkey,
      ) ?? null
    );
  }, [managedAgentsQuery.data, metadataDefaultAgentPubkey]);

  const messageQuery = useChannelMessagesQuery(selectedChat);
  useChannelSubscription(selectedChat);
  const messages = messageQuery.data ?? [];
  const pubkeys = React.useMemo(
    () => [
      ...new Set(
        [
          identityQuery.data?.pubkey,
          defaultAgent?.pubkey,
          ...messages.map((message) => message.pubkey),
        ]
          .filter((value): value is string => Boolean(value))
          .map((value) => value.toLowerCase()),
      ),
    ],
    [defaultAgent?.pubkey, identityQuery.data?.pubkey, messages],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const sendMessageMutation = useSendMessageMutation(
    selectedChat,
    identityQuery.data,
  );
  const archiveChatMutation = useArchiveChatMutation();

  // Latest PR link ANY agent in the chat posted — drives the header toggle
  // and the top-right work module. Matching only the default agent missed
  // PRs from additional agents added to the chat.
  const agentPullRequestHref = React.useMemo(() => {
    const agentKeys = new Set(
      (managedAgentsQuery.data ?? []).map((agent) =>
        normalizePubkey(agent.pubkey),
      ),
    );
    const configuredAgent =
      metadata?.defaultAgentPubkey ?? defaultAgent?.pubkey;
    if (configuredAgent) {
      agentKeys.add(normalizePubkey(configuredAgent));
    }
    if (agentKeys.size === 0) {
      return null;
    }
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index];
      if (!agentKeys.has(normalizePubkey(message.pubkey))) {
        continue;
      }
      const preview = extractSupportedLinkPreviews(message.content).find(
        (candidate) => candidate.kind === "github-pull-request",
      );
      if (preview) {
        return preview.href;
      }
    }
    return null;
  }, [
    defaultAgent?.pubkey,
    managedAgentsQuery.data,
    messages,
    metadata?.defaultAgentPubkey,
  ]);
  // null = auto: open once the agent has produced a PR, closed otherwise.
  // A click stores an explicit preference for the current chat.
  const [workPanelPreference, setWorkPanelPreference] = React.useState<
    boolean | null
  >(null);
  const workPanelChatRef = React.useRef(selectedChatId);
  if (workPanelChatRef.current !== selectedChatId) {
    workPanelChatRef.current = selectedChatId;
    setWorkPanelPreference(null);
  }
  const isWorkPanelOpen = workPanelPreference ?? agentPullRequestHref !== null;
  const startManagedAgentMutation = useStartManagedAgentMutation();
  const [isEnsuringDefaultAgent, setIsEnsuringDefaultAgent] =
    React.useState(false);
  const handleArchiveChat = React.useCallback(
    async (chatId: string) => {
      try {
        await archiveChatMutation.mutateAsync(chatId);
        toast.success("Chat archived");
        if (selectedChatId === chatId) {
          void goChats({ replace: true });
        }
      } catch (error) {
        toast.error("Could not archive chat", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [archiveChatMutation, goChats, selectedChatId],
  );

  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const pinnedChatIds = useStoredChatPins(activeWorkspace?.id);
  const handleTogglePin = React.useCallback(
    (chatId: string) => {
      toggleStoredChatPin(activeWorkspace?.id, chatId);
    },
    [activeWorkspace?.id],
  );
  const [renamingChatId, setRenamingChatId] = React.useState<string | null>(
    null,
  );
  const renamingChat =
    renamingChatId !== null
      ? (chats.find((chat) => chat.id === renamingChatId) ?? null)
      : null;
  const renamingMetadata = renamingChatId
    ? (metadataByChatId.get(renamingChatId) ?? null)
    : null;
  const handleRenameChat = React.useCallback(
    async (title: string) => {
      if (!renamingChat) {
        return;
      }
      const metadata = metadataByChatId.get(renamingChat.id) ?? null;
      try {
        await updateMetadataMutation.mutateAsync({
          channelId: renamingChat.id,
          title,
          defaultAgentPubkey: metadata?.defaultAgentPubkey ?? undefined,
          templateId: metadata?.templateId ?? undefined,
          projectId: metadata?.projectId ?? undefined,
          projectName: metadata?.projectName ?? undefined,
          projectPath: metadata?.projectPath ?? undefined,
          projectTemplateId: metadata?.projectTemplateId ?? undefined,
          source: metadata?.sourceChannelId
            ? {
                channelId: metadata.sourceChannelId,
                eventId: metadata.sourceEventId ?? undefined,
                threadRootId: metadata.sourceThreadRootId ?? undefined,
              }
            : undefined,
        });
        setRenamingChatId(null);
      } catch (error) {
        toast.error("Could not rename chat", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [metadataByChatId, renamingChat, updateMetadataMutation],
  );
  const ensuredChatIdsRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    if (!selectedChat || metadataQuery.isLoading) {
      return;
    }
    if (metadata?.defaultAgentPubkey) {
      return;
    }
    if (ensuredChatIdsRef.current.has(selectedChat.id)) {
      return;
    }
    ensuredChatIdsRef.current.add(selectedChat.id);
    void ensureWelcomeGuideAgentInChannel(
      selectedChat.id,
      activeWorkspace?.relayUrl,
    )
      .then((agent) =>
        updateMetadataMutation.mutateAsync({
          channelId: selectedChat.id,
          defaultAgentPubkey: agent.pubkey,
          title: metadata?.title ?? selectedChat.name,
          templateId: metadata?.templateId ?? undefined,
          projectId: metadata?.projectId ?? undefined,
          projectName: metadata?.projectName ?? undefined,
          projectPath: metadata?.projectPath ?? undefined,
          projectTemplateId: metadata?.projectTemplateId ?? undefined,
          source: metadata?.sourceChannelId
            ? {
                channelId: metadata.sourceChannelId,
                eventId: metadata.sourceEventId ?? undefined,
                threadRootId: metadata.sourceThreadRootId ?? undefined,
              }
            : undefined,
        }),
      )
      .catch((error) => {
        console.error("Failed to ensure Fizz for chat", selectedChat.id, error);
      });
  }, [
    activeWorkspace?.relayUrl,
    metadata,
    metadataQuery.isLoading,
    selectedChat,
    updateMetadataMutation,
  ]);

  React.useEffect(() => {
    if (!selectedChat || messages.length === 0) {
      return;
    }
    const lastMessage = messages[messages.length - 1];
    if (!lastMessage) {
      return;
    }
    markChannelRead(
      selectedChat.id,
      new Date(lastMessage.created_at * 1_000).toISOString(),
    );
  }, [markChannelRead, messages, selectedChat]);

  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const defaultAgentPubkey =
        metadata?.defaultAgentPubkey ?? defaultAgent?.pubkey ?? null;
      await sendMessageMutation.mutateAsync({
        content,
        mentionPubkeys: uniqueMentionPubkeys(
          identityQuery.data?.pubkey,
          mentionPubkeys,
          defaultAgentPubkey,
        ),
        mediaTags,
      });
    },
    [
      defaultAgent?.pubkey,
      identityQuery.data?.pubkey,
      metadata?.defaultAgentPubkey,
      sendMessageMutation,
    ],
  );

  const handleActivateAgent = React.useCallback(async () => {
    if (!selectedChat) {
      return;
    }

    setIsEnsuringDefaultAgent(true);
    try {
      if (defaultAgent) {
        await addBotToChat(selectedChat.id, defaultAgent.pubkey);
        if (
          defaultAgent.status !== "running" &&
          defaultAgent.status !== "deployed"
        ) {
          const updatedAgent = await startManagedAgentMutation.mutateAsync(
            defaultAgent.pubkey,
          );
          toast.success(`${updatedAgent.name || defaultAgent.name} activated`);
        } else {
          await managedAgentsQuery.refetch();
          toast.success(`${defaultAgent.name} activated`);
        }
        return;
      }

      const agent = await ensureWelcomeGuideAgentInChannel(
        selectedChat.id,
        activeWorkspace?.relayUrl,
      );
      await updateMetadataMutation.mutateAsync({
        channelId: selectedChat.id,
        defaultAgentPubkey: agent.pubkey,
        title: metadata?.title ?? selectedChat.name,
        templateId: metadata?.templateId ?? undefined,
        projectId: metadata?.projectId ?? undefined,
        projectName: metadata?.projectName ?? undefined,
        projectPath: metadata?.projectPath ?? undefined,
        projectTemplateId: metadata?.projectTemplateId ?? undefined,
        source: metadata?.sourceChannelId
          ? {
              channelId: metadata.sourceChannelId,
              eventId: metadata.sourceEventId ?? undefined,
              threadRootId: metadata.sourceThreadRootId ?? undefined,
            }
          : undefined,
      });
      await managedAgentsQuery.refetch();
      toast.success(`${agent.name || "Fizz"} activated`);
    } catch (error) {
      console.error("Failed to activate chat agent", error);
      toast.error("Could not activate agent", {
        description: error instanceof Error ? error.message : undefined,
      });
    } finally {
      setIsEnsuringDefaultAgent(false);
    }
  }, [
    activeWorkspace?.relayUrl,
    defaultAgent,
    managedAgentsQuery,
    metadata,
    selectedChat,
    startManagedAgentMutation,
    updateMetadataMutation,
  ]);

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[17rem_minmax(0,1fr)] overflow-hidden">
      <aside className="min-h-0 border-r border-border/60 bg-muted/15">
        <ChatList
          chats={chats}
          getChannelReadAt={getChannelReadAt}
          identityPubkey={identityPubkey}
          isLoading={chatsQuery.isLoading || metadataListQuery.isLoading}
          metadataByChatId={metadataByChatId}
          onRenameChat={setRenamingChatId}
          onTogglePin={handleTogglePin}
          pinnedChatIds={pinnedChatIds}
          onCreateChat={() => void goChats({ projectId: null })}
          onCreateProjectChat={(projectId) =>
            void goChats({ projectId, replace: true })
          }
          onArchiveChat={(chatId) => void handleArchiveChat(chatId)}
          onSelectChat={(chatId) => void goChat(chatId)}
          onUpdateProject={(project) =>
            upsertStoredChatProject(activeWorkspace?.id, project)
          }
          archivingChatId={
            archiveChatMutation.isPending
              ? (archiveChatMutation.variables ?? null)
              : null
          }
          projects={chatProjects}
          readStateVersion={readStateVersion}
          selectedChatId={selectedChatId}
          templates={templates}
          unreadChannelCounts={unreadChannelCounts}
          unreadChannelIds={unreadChannelIds}
        />
      </aside>

      <main className="flex min-h-0 min-w-0 flex-col bg-background">
        {selectedChat ? (
          <ChatDetail
            chat={selectedChat}
            defaultAgent={defaultAgent}
            identityPubkey={identityPubkey}
            isActivatingAgent={
              isEnsuringDefaultAgent || startManagedAgentMutation.isPending
            }
            isLoadingMessages={messageQuery.isLoading}
            isSending={sendMessageMutation.isPending}
            messages={messages}
            metadata={metadata}
            onActivateAgent={handleActivateAgent}
            onProjectCreated={(project) =>
              upsertStoredChatProject(activeWorkspace?.id, project)
            }
            onSend={handleSend}
            profiles={profiles}
            projects={chatProjects}
            showWorkPanel={isWorkPanelOpen}
            workPanelHref={agentPullRequestHref}
            shareAction={
              <ChatHeaderActions
                chat={selectedChat}
                defaultAgentPubkey={defaultAgent?.pubkey}
                messages={messages}
                metadata={metadata}
                workPanelToggle={
                  <Button
                    aria-label={
                      isWorkPanelOpen ? "Hide work panel" : "Show work panel"
                    }
                    aria-pressed={isWorkPanelOpen}
                    data-testid="toggle-work-panel"
                    onClick={() => setWorkPanelPreference(!isWorkPanelOpen)}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <GitPullRequest
                      className={cn(
                        "h-4 w-4",
                        isWorkPanelOpen && "text-primary",
                      )}
                    />
                  </Button>
                }
              />
            }
            templates={templates}
          />
        ) : (
          <QuickStartChat
            initialProjectId={initialProjectId}
            projects={chatProjects}
            relayUrl={activeWorkspace?.relayUrl}
            onProjectCreated={(project) =>
              upsertStoredChatProject(activeWorkspace?.id, project)
            }
            onCreated={(chat) => void goChat(chat.id, { replace: true })}
          />
        )}
      </main>
      <ChatRenameDialog
        currentTitle={
          renamingMetadata?.title?.trim() || renamingChat?.name || ""
        }
        isSaving={updateMetadataMutation.isPending}
        onOpenChange={(open) => {
          if (!open) {
            setRenamingChatId(null);
          }
        }}
        onRename={(title) => void handleRenameChat(title)}
        open={renamingChat !== null}
      />
    </div>
  );
}
