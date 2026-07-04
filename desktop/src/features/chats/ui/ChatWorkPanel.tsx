import { GitBranch } from "lucide-react";

import {
  parseGithubPullRequestRef,
  useGithubPullRequestQuery,
} from "@/shared/lib/githubPullRequest";
import { parseSupportedLinkPreview } from "@/shared/lib/linkPreview";
import { cn } from "@/shared/lib/cn";
import { GithubPullRequestCard } from "@/shared/ui/link-preview-attachment";

/**
 * Right-hand work module for a chat whose agent produced a pull request:
 * the PR's source branch and the live PR card (status, diff stats, link),
 * grouped in a secondary-surface container. The drawer eases open/closed on
 * its width so the conversation column and composer slide to make room.
 */
export function ChatWorkPanel({
  open = true,
  prHref,
}: {
  open?: boolean;
  prHref: string;
}) {
  const preview = parseSupportedLinkPreview(prHref);
  const ref = parseGithubPullRequestRef(prHref);
  const query = useGithubPullRequestQuery(ref);
  const branch = query.data?.headRef?.trim();

  if (!preview) {
    return null;
  }

  return (
    <aside
      aria-hidden={!open}
      className={cn(
        "flex shrink-0 flex-col overflow-hidden transition-[width,opacity] duration-300 ease-out",
        open ? "w-96 opacity-100" : "pointer-events-none w-0 opacity-0",
      )}
      data-testid="chat-work-panel"
    >
      {/* Fixed-width inner wrapper so content never reflows mid-slide. */}
      <div className="w-96 overflow-y-auto py-4 pl-1 pr-4">
        <div className="flex flex-col gap-2">
          {branch ? (
            // Same attachment styling as the generic link chips.
            <div className="flex items-center gap-1.5 rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-xs">
              <GitBranch className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 truncate font-mono">{branch}</span>
            </div>
          ) : null}
          <GithubPullRequestCard className="w-full" preview={preview} />
        </div>
      </div>
    </aside>
  );
}
