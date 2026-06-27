import type { ReactNode } from "react";
import { X } from "lucide-react";

import { CopyButton } from "@/features/agents/ui/CopyButton";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  PROFILE_PANEL_VIEW_TITLES,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { Button } from "@/shared/ui/button";

export function getUserProfilePanelHeaderContent({
  agentSettingsMenu,
  effectivePubkey,
  logCopyValue,
  logSubtitle,
  onBack,
  onClose,
  view,
  viewerIsOwner,
}: {
  agentSettingsMenu: ReactNode;
  effectivePubkey: string | null;
  logCopyValue?: string | null;
  logSubtitle?: string | null;
  onBack: () => void;
  onClose: () => void;
  view: ProfilePanelView;
  viewerIsOwner: boolean;
}) {
  const title = PROFILE_PANEL_VIEW_TITLES[view];
  const shouldShowLogDetails =
    (view === "diagnostics" || view === "logs") && Boolean(logSubtitle);
  const headerLeftContent = (
    <AuxiliaryPanelHeaderGroup
      backButtonAriaLabel="Back to profile"
      backButtonTestId="user-profile-panel-back"
      className={shouldShowLogDetails ? "items-start" : undefined}
      onBack={view !== "summary" ? onBack : undefined}
    >
      {shouldShowLogDetails ? (
        <div className="min-w-0 flex-1">
          <AuxiliaryPanelTitle className="translate-y-0 leading-5">
            {title}
          </AuxiliaryPanelTitle>
          <p
            className="min-w-0 truncate font-mono text-2xs text-muted-foreground"
            title={logSubtitle ?? undefined}
          >
            {logSubtitle}
          </p>
        </div>
      ) : (
        <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>
      )}
    </AuxiliaryPanelHeaderGroup>
  );
  const headerActions = (
    <AuxiliaryPanelHeaderActions>
      {view === "memories" && viewerIsOwner && effectivePubkey ? (
        <MemoryRefreshButton
          agentPubkey={effectivePubkey}
          variant="outline"
          viewerIsOwner={viewerIsOwner}
        />
      ) : null}
      {view === "summary" ? agentSettingsMenu : null}
      {shouldShowLogDetails ? (
        <CopyButton
          className="text-muted-foreground hover:text-foreground"
          iconOnly
          label="Copy log"
          size="icon"
          value={logCopyValue ?? ""}
          variant="ghost"
        />
      ) : null}
      <Button
        aria-label="Close profile"
        data-testid="user-profile-panel-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </AuxiliaryPanelHeaderActions>
  );

  return { headerActions, headerLeftContent };
}
