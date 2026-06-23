import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Clipboard,
  Ellipsis,
  FileText,
  OctagonX,
  Pencil,
  Play,
  Power,
  Square,
  Trash2,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type {
  AgentPersona,
  ManagedAgent,
  PresenceLookup,
} from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { EditAgentDialog } from "./EditAgentDialog";

type UnifiedAgentsSectionProps = {
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  channelIdToName: Record<string, string>;
  channelsByPubkey: Record<string, { id: string; name: string }[]>;
  agentsError: Error | null;
  isActionPending: boolean;
  isAgentsLoading: boolean;
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onAddToChannel: (agent: ManagedAgent) => void;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
  onCreateAgent: () => void;
  onDeleteAgent: (pubkey: string) => void;
  onOpenAgentProfile?: (pubkey: string) => void;
  onSelectLogAgent: (pubkey: string | null) => void;
  onStartAgent: (pubkey: string) => void;
  onStopAgent: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
  selectedLogAgentPubkey: string | null;
  canChooseCatalog: boolean;
  personas: AgentPersona[];
  personasError: Error | null;
  personaFeedbackErrorMessage: string | null;
  personaFeedbackNoticeMessage: string | null;
  isPersonasLoading: boolean;
  isPersonasPending: boolean;
  onCreatePersona: () => void;
  onChooseCatalog: () => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onExportPersona: (persona: AgentPersona) => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onImportPersonaFile: (fileBytes: number[], fileName: string) => void;
};

type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

const AGENT_CARD_COLUMN_CLASS = "w-full";
const AGENT_CARD_GRID_CLASS = `${AGENT_CARD_COLUMN_CLASS} grid grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3`;

function buildUnifiedGroups(personas: AgentPersona[], agents: ManagedAgent[]) {
  const byPersonaId = new Map<string, ManagedAgent[]>();
  const ungrouped: ManagedAgent[] = [];

  for (const agent of agents) {
    if (!agent.personaId) {
      ungrouped.push(agent);
    } else {
      const list = byPersonaId.get(agent.personaId) ?? [];
      list.push(agent);
      byPersonaId.set(agent.personaId, list);
    }
  }

  const matched = new Set<string>();
  const groups: PersonaGroup[] = personas.map((p) => {
    matched.add(p.id);
    return { persona: p, agents: byPersonaId.get(p.id) ?? [] };
  });

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) unknown.push(...list);
  }

  return { groups, ungrouped, unknown };
}

export function UnifiedAgentsSection(props: UnifiedAgentsSectionProps) {
  const {
    actionErrorMessage,
    actionNoticeMessage,
    agents,
    agentsError,
    isActionPending,
    isAgentsLoading,
    onAddToChannel,
    onBulkRemoveStopped,
    onBulkStopRunning,
    onCreateAgent,
    onDeleteAgent,
    onOpenAgentProfile,
    onSelectLogAgent,
    onStartAgent,
    onStopAgent,
    onToggleStartOnAppLaunch,
    canChooseCatalog,
    personas,
    personasError,
    personaFeedbackErrorMessage,
    personaFeedbackNoticeMessage,
    isPersonasLoading,
    isPersonasPending,
    onCreatePersona,
    onChooseCatalog,
    onDuplicatePersona,
    onEditPersona,
    onExportPersona,
    onDeactivatePersona,
    onDeletePersona,
    onImportPersonaFile,
  } = props;

  const runningCount = agents.filter((agent) =>
    isManagedAgentActive(agent),
  ).length;
  const stoppedCount = agents.filter(
    (agent) => agent.status === "stopped" || agent.status === "not_deployed",
  ).length;
  const { groups, ungrouped, unknown } = React.useMemo(
    () => buildUnifiedGroups(personas, agents),
    [personas, agents],
  );
  const additionalPersonaAgents = React.useMemo(() => {
    const additional: ManagedAgent[] = [];
    for (const group of groups) {
      const primary = pickProfileAgent(group.agents);
      for (const agent of group.agents) {
        if (primary?.pubkey !== agent.pubkey) {
          additional.push(agent);
        }
      }
    }
    return additional;
  }, [groups]);
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set());
  const {
    fileInputRef,
    isDragOver,
    dropHandlers,
    handleFileChange,
    openFilePicker,
  } = useFileImportZone({ onImportFile: onImportPersonaFile });

  function toggle(key: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  useFeedbackToasts(actionNoticeMessage, actionErrorMessage);
  useFeedbackToasts(personaFeedbackNoticeMessage, personaFeedbackErrorMessage);
  const isLoading = isAgentsLoading || isPersonasLoading;
  const agentMenuProps = {
    isActionPending,
    onAddToChannel,
    onDelete: onDeleteAgent,
    onOpenLogs: onSelectLogAgent,
    onStart: onStartAgent,
    onStop: onStopAgent,
    onToggleStartOnAppLaunch,
  } as const;
  const personaMenuProps = {
    isActionPending,
    isPersonasPending,
    onDeactivatePersona,
    onDeletePersona,
    onDuplicatePersona,
    onEditPersona,
    onExportPersona,
  } as const;

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-personas"
      {...dropHandlers}
    >
      {isDragOver ? (
        <div className="pointer-events-none absolute -inset-1 z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/50 bg-background/80 backdrop-blur-sm">
          <p className="text-sm font-medium text-primary">
            Drop .persona.md, .persona.json, .persona.png, or .zip to import
          </p>
        </div>
      ) : null}

      <SectionHeader
        agentCount={agents.length}
        fileInputRef={fileInputRef}
        handleFileChange={handleFileChange}
        isActionPending={isActionPending}
        runningCount={runningCount}
        stoppedCount={stoppedCount}
        onBulkRemoveStopped={onBulkRemoveStopped}
        onBulkStopRunning={onBulkStopRunning}
      />

      {isLoading ? <LoadingSkeleton /> : null}

      {!isLoading ? (
        <div className="space-y-3" data-testid="unified-agents-groups">
          <div className={AGENT_CARD_GRID_CLASS}>
            {groups.map((group) => {
              const profileAgent = pickProfileAgent(group.agents);
              return (
                <AgentPersonaCard
                  agent={profileAgent}
                  agentMenuProps={agentMenuProps}
                  key={group.persona.id}
                  persona={group.persona}
                  personaMenuProps={personaMenuProps}
                  onOpenAgentProfile={onOpenAgentProfile}
                />
              );
            })}
            <NewAgentCard
              canChooseCatalog={canChooseCatalog}
              isPersonasPending={isPersonasPending}
              openFilePicker={openFilePicker}
              onChooseCatalog={onChooseCatalog}
              onCreateAgent={onCreateAgent}
              onCreatePersona={onCreatePersona}
            />
          </div>

          {additionalPersonaAgents.length > 0 ? (
            <CollapsibleAgentGroup
              agents={additionalPersonaAgents}
              agentMenuProps={agentMenuProps}
              collapsed={collapsed}
              groupKey="__additional_persona_agents__"
              label="Additional agent instances"
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
          {unknown.length > 0 ? (
            <CollapsibleAgentGroup
              agents={unknown}
              agentMenuProps={agentMenuProps}
              collapsed={collapsed}
              groupKey="__unknown__"
              label="Unknown Persona"
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              agentMenuProps={agentMenuProps}
              collapsed={collapsed}
              groupKey="__ungrouped__"
              label="Custom Agents"
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
        </div>
      ) : null}

      {!isLoading && stoppedCount > 0 ? (
        <div
          className={`${AGENT_CARD_COLUMN_CLASS} flex items-center justify-between rounded-xl border border-border/60 bg-muted/30 px-4 py-2.5`}
        >
          <p className="text-sm text-muted-foreground">
            {stoppedCount} stopped {stoppedCount === 1 ? "agent" : "agents"}
          </p>
          <Button
            className="text-destructive"
            disabled={isActionPending}
            onClick={onBulkRemoveStopped}
            size="sm"
            variant="ghost"
          >
            <Trash2 className="mr-1.5 h-4 w-4" />
            Remove stopped
          </Button>
        </div>
      ) : null}

      {agentsError ? (
        <p
          className={`${AGENT_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {agentsError.message}
        </p>
      ) : null}
      {personasError ? (
        <p
          className={`${AGENT_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {personasError.message}
        </p>
      ) : null}
    </section>
  );
}

function pickProfileAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const activeDiff =
      Number(isManagedAgentActive(right)) - Number(isManagedAgentActive(left));
    if (activeDiff !== 0) return activeDiff;
    return left.name.localeCompare(right.name);
  })[0];
}

type AgentMenuProps = {
  isActionPending: boolean;
  onAddToChannel: (agent: ManagedAgent) => void;
  onDelete: (pubkey: string) => void;
  onOpenLogs: (pubkey: string) => void;
  onStart: (pubkey: string) => void;
  onStop: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
};

type PersonaMenuProps = {
  isActionPending: boolean;
  isPersonasPending: boolean;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onExportPersona: (persona: AgentPersona) => void;
};

function AgentPersonaCard({
  agent,
  agentMenuProps,
  persona,
  personaMenuProps,
  onOpenAgentProfile,
}: {
  agent: ManagedAgent | undefined;
  agentMenuProps: AgentMenuProps;
  persona: AgentPersona;
  personaMenuProps: PersonaMenuProps;
  onOpenAgentProfile?: (pubkey: string) => void;
}) {
  const title = persona.displayName;
  const modelLabel = formatAgentModelLabel(agent?.model ?? persona.model);
  const profileQuery = useUserProfileQuery(agent?.pubkey);
  const avatarUrl = agent
    ? firstAvatarUrl(profileQuery.data?.avatarUrl, persona.avatarUrl)
    : persona.avatarUrl;

  return (
    <AgentIdentityCard
      actions={
        <AgentPersonaActionsMenu
          agent={agent}
          agentMenuProps={agentMenuProps}
          persona={persona}
          personaMenuProps={personaMenuProps}
        />
      }
      ariaLabel={`${title} agent profile`}
      avatarUrl={avatarUrl}
      dataTestId={`persona-agent-row-${persona.id}`}
      label={title}
      modelLabel={modelLabel}
      onClick={() => {
        if (agent && onOpenAgentProfile) {
          onOpenAgentProfile(agent.pubkey);
          return;
        }
        personaMenuProps.onEditPersona(persona);
      }}
    />
  );
}

function StandaloneAgentCard({
  agent,
  agentMenuProps,
  onOpenAgentProfile,
}: {
  agent: ManagedAgent;
  agentMenuProps: AgentMenuProps;
  onOpenAgentProfile?: (pubkey: string) => void;
}) {
  const title = agent.name;
  const profileQuery = useUserProfileQuery(agent.pubkey);

  return (
    <AgentIdentityCard
      actions={<AgentActionsMenu agent={agent} {...agentMenuProps} />}
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      label={title}
      modelLabel={formatAgentModelLabel(agent.model)}
      onClick={() => {
        if (onOpenAgentProfile) {
          onOpenAgentProfile(agent.pubkey);
        } else {
          agentMenuProps.onOpenLogs(agent.pubkey);
        }
      }}
    />
  );
}

function AgentPersonaActionsMenu({
  agent,
  agentMenuProps,
  persona,
  personaMenuProps,
}: {
  agent: ManagedAgent | undefined;
  agentMenuProps: AgentMenuProps;
  persona: AgentPersona;
  personaMenuProps: PersonaMenuProps;
}) {
  const [editOpen, setEditOpen] = React.useState(false);
  const disabled =
    personaMenuProps.isActionPending || personaMenuProps.isPersonasPending;

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            aria-label={`Open actions for ${persona.displayName}`}
            className="flex h-7 w-7 items-center justify-center rounded-md bg-background/70 text-muted-foreground transition-colors hover:bg-background hover:text-foreground data-[state=open]:bg-background data-[state=open]:text-foreground"
            type="button"
          >
            <Ellipsis className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          {agent ? (
            <>
              <AgentActionItems
                agent={agent}
                {...agentMenuProps}
                onEdit={() => setEditOpen(true)}
              />
              <DropdownMenuSeparator />
            </>
          ) : null}
          {!persona.isBuiltIn ? (
            <DropdownMenuItem
              disabled={disabled}
              onClick={() => personaMenuProps.onEditPersona(persona)}
            >
              <Pencil className="h-4 w-4" />
              Edit persona
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuItem
            disabled={disabled}
            onClick={() => personaMenuProps.onDuplicatePersona(persona)}
          >
            <Clipboard className="h-4 w-4" />
            Duplicate persona
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={disabled}
            onClick={() => personaMenuProps.onExportPersona(persona)}
          >
            <FileText className="h-4 w-4" />
            Export persona
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {persona.isBuiltIn ? (
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              disabled={disabled}
              onClick={() => personaMenuProps.onDeactivatePersona(persona)}
            >
              <Trash2 className="h-4 w-4" />
              Remove from My Agents
            </DropdownMenuItem>
          ) : persona.sourceTeam ? (
            <DropdownMenuItem disabled>
              <Trash2 className="h-4 w-4" />
              Managed by team
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              disabled={disabled}
              onClick={() => personaMenuProps.onDeletePersona(persona)}
            >
              <Trash2 className="h-4 w-4" />
              Delete persona
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      {agent ? (
        <EditAgentDialog
          agent={agent}
          onOpenChange={setEditOpen}
          open={editOpen}
        />
      ) : null}
    </>
  );
}

function AgentActionsMenu({
  agent,
  isActionPending,
  onAddToChannel,
  onDelete,
  onOpenLogs,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
}: { agent: ManagedAgent } & AgentMenuProps) {
  const [editOpen, setEditOpen] = React.useState(false);

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            aria-label={`Agent actions for ${agent.name}`}
            className="flex h-7 w-7 items-center justify-center rounded-md bg-background/70 text-muted-foreground transition-colors hover:bg-background hover:text-foreground data-[state=open]:bg-background data-[state=open]:text-foreground"
            data-testid={`managed-agent-actions-${agent.pubkey}`}
            type="button"
          >
            <Ellipsis className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          <AgentActionItems
            agent={agent}
            isActionPending={isActionPending}
            onAddToChannel={onAddToChannel}
            onDelete={onDelete}
            onOpenLogs={onOpenLogs}
            onStart={onStart}
            onStop={onStop}
            onToggleStartOnAppLaunch={onToggleStartOnAppLaunch}
            onEdit={() => setEditOpen(true)}
          />
        </DropdownMenuContent>
      </DropdownMenu>

      <EditAgentDialog
        agent={agent}
        onOpenChange={setEditOpen}
        open={editOpen}
      />
    </>
  );
}

function AgentActionItems({
  agent,
  isActionPending,
  onAddToChannel,
  onDelete,
  onEdit,
  onOpenLogs,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
}: { agent: ManagedAgent; onEdit?: () => void } & AgentMenuProps) {
  const isActive = isManagedAgentActive(agent);

  return (
    <>
      {agent.backend.type === "provider" ? (
        <>
          <DropdownMenuItem
            disabled={isActionPending}
            onClick={() => onStart(agent.pubkey)}
          >
            <Play className="h-4 w-4" />
            {isActive ? "Redeploy" : "Deploy"}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={isActionPending}
            onClick={() => onStop(agent.pubkey)}
          >
            <Square className="h-4 w-4" />
            Shutdown
          </DropdownMenuItem>
        </>
      ) : isActive ? (
        <DropdownMenuItem
          disabled={isActionPending}
          onClick={() => onStop(agent.pubkey)}
        >
          <Square className="h-4 w-4" />
          Stop
        </DropdownMenuItem>
      ) : (
        <DropdownMenuItem
          disabled={isActionPending}
          onClick={() => onStart(agent.pubkey)}
        >
          <Play className="h-4 w-4" />
          Spawn
        </DropdownMenuItem>
      )}

      {agent.backend.type !== "provider" && onEdit ? (
        <DropdownMenuItem onClick={onEdit}>
          <Pencil className="h-4 w-4" />
          Edit agent
        </DropdownMenuItem>
      ) : null}

      <DropdownMenuItem
        disabled={isActionPending}
        onClick={() => onAddToChannel(agent)}
      >
        <UserPlus className="h-4 w-4" />
        Add to channel
      </DropdownMenuItem>

      <DropdownMenuItem
        onClick={async () => {
          await navigator.clipboard.writeText(agent.pubkey);
          toast.success("Copied pubkey to clipboard");
        }}
      >
        <Clipboard className="h-4 w-4" />
        Copy pubkey
      </DropdownMenuItem>

      {agent.backend.type === "local" ? (
        <DropdownMenuItem onClick={() => onOpenLogs(agent.pubkey)}>
          <FileText className="h-4 w-4" />
          View logs
        </DropdownMenuItem>
      ) : null}

      {agent.backend.type === "local" ? (
        <DropdownMenuItem
          disabled={isActionPending}
          onClick={() =>
            onToggleStartOnAppLaunch(agent.pubkey, !agent.startOnAppLaunch)
          }
        >
          <Power className="h-4 w-4" />
          {agent.startOnAppLaunch ? "Disable auto-start" : "Enable auto-start"}
        </DropdownMenuItem>
      ) : null}

      <DropdownMenuSeparator />

      <DropdownMenuItem
        className="text-destructive focus:text-destructive"
        disabled={isActionPending}
        onClick={() => onDelete(agent.pubkey)}
      >
        <Trash2 className="h-4 w-4" />
        Delete
      </DropdownMenuItem>
    </>
  );
}

function formatAgentModelLabel(model: string | null | undefined) {
  const trimmed = model?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "Auto";
}

function firstAvatarUrl(
  ...candidates: Array<string | null | undefined>
): string | null {
  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed) return trimmed;
  }
  return null;
}

function SectionHeader({
  agentCount,
  fileInputRef,
  handleFileChange,
  isActionPending,
  runningCount,
  stoppedCount,
  onBulkRemoveStopped,
  onBulkStopRunning,
}: {
  agentCount: number;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  handleFileChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
  isActionPending: boolean;
  runningCount: number;
  stoppedCount: number;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
}) {
  return (
    <div
      className={`${AGENT_CARD_COLUMN_CLASS} flex items-center justify-between gap-3`}
    >
      <div>
        <h3 className="text-sm font-semibold tracking-tight">Your Agents</h3>
        <p className="text-sm text-secondary-foreground/75">
          Agents in this workspace.
        </p>
      </div>
      <input
        accept=".md,.json,.png,.zip"
        className="hidden"
        onChange={handleFileChange}
        ref={fileInputRef}
        type="file"
      />
      {agentCount > 0 ? (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Bulk actions"
              className="h-7 w-7"
              size="icon"
              variant="ghost"
            >
              <Ellipsis className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => event.preventDefault()}
          >
            <DropdownMenuItem
              disabled={isActionPending || runningCount === 0}
              onClick={onBulkStopRunning}
            >
              <OctagonX className="h-4 w-4" />
              Stop all running ({runningCount})
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              disabled={isActionPending || stoppedCount === 0}
              onClick={onBulkRemoveStopped}
            >
              <Trash2 className="h-4 w-4" />
              Remove all stopped ({stoppedCount})
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}

function NewAgentCard({
  canChooseCatalog,
  isPersonasPending,
  openFilePicker,
  onChooseCatalog,
  onCreateAgent,
  onCreatePersona,
}: {
  canChooseCatalog: boolean;
  isPersonasPending: boolean;
  openFilePicker: () => void;
  onChooseCatalog: () => void;
  onCreateAgent: () => void;
  onCreatePersona: () => void;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <CreateIdentityCard
          ariaLabel="New agent"
          dataTestId="new-agent-card"
          label="New agent"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem
          disabled={isPersonasPending}
          onClick={onCreatePersona}
        >
          Persona
        </DropdownMenuItem>
        {canChooseCatalog ? (
          <DropdownMenuItem
            disabled={isPersonasPending}
            onClick={onChooseCatalog}
          >
            Choose from Catalog...
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onCreateAgent}>
          Custom Agent
        </DropdownMenuItem>
        <DropdownMenuItem onClick={openFilePicker}>
          Import persona file
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function LoadingSkeleton() {
  return (
    <div className={AGENT_CARD_GRID_CLASS}>
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-14"
        footerTitleWidthClass="w-24"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-20"
        footerTitleWidthClass="w-32"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-16"
        footerTitleWidthClass="w-28"
      />
    </div>
  );
}

function CollapsibleAgentGroup({
  groupKey,
  label,
  agents,
  agentMenuProps,
  collapsed,
  onToggle,
  onOpenAgentProfile,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  agentMenuProps: AgentMenuProps;
  collapsed: ReadonlySet<string>;
  onToggle: (key: string) => void;
  onOpenAgentProfile?: (pubkey: string) => void;
}) {
  const isCollapsed = collapsed.has(groupKey);
  return (
    <div className={`${AGENT_CARD_COLUMN_CLASS} space-y-2`}>
      <button
        className="group flex items-center gap-2 rounded-md px-1 py-1 text-left transition-colors hover:bg-muted/50"
        onClick={() => onToggle(groupKey)}
        type="button"
      >
        {isCollapsed ? (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
        ) : (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <span className="text-sm font-medium">{label}</span>
        <span className="text-xs text-muted-foreground">({agents.length})</span>
      </button>
      {!isCollapsed ? (
        <div className={AGENT_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <StandaloneAgentCard
              agent={agent}
              agentMenuProps={agentMenuProps}
              key={agent.pubkey}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
