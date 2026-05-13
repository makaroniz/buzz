import { X } from "lucide-react";

import { Button } from "@/shared/ui/button";

type MessageComposerReplyTargetProps = {
  author: string;
  body: string;
  onCancelReply?: () => void;
};

export function MessageComposerReplyTarget({
  author,
  body,
  onCancelReply,
}: MessageComposerReplyTargetProps) {
  return (
    <div
      className="mb-3 flex items-start justify-between gap-3 rounded-2xl border border-border/70 bg-muted/40 px-3 py-2"
      data-testid="reply-target"
    >
      <div className="min-w-0">
        <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
          Replying to {author}
        </p>
        <p className="truncate text-sm text-foreground/80">{body}</p>
      </div>
      <Button
        aria-label="Cancel reply"
        className="h-7 w-7 shrink-0 px-0"
        onClick={onCancelReply}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X className="h-4 w-4" />
      </Button>
    </div>
  );
}
