import { X } from "lucide-react";

import { Button } from "@/shared/ui/button";

type MessageComposerEditTargetProps = {
  body: string;
  onCancelEdit?: () => void;
};

export function MessageComposerEditTarget({
  body,
  onCancelEdit,
}: MessageComposerEditTargetProps) {
  return (
    <div
      className="mb-3 flex items-start justify-between gap-3 rounded-2xl border border-primary/30 bg-primary/5 px-3 py-2"
      data-testid="edit-target"
    >
      <div className="min-w-0">
        <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
          Editing message
        </p>
        <p className="truncate text-sm text-foreground/80">{body}</p>
      </div>
      <Button
        aria-label="Cancel edit"
        className="h-7 w-7 shrink-0 px-0"
        onClick={onCancelEdit}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X className="h-4 w-4" />
      </Button>
    </div>
  );
}
