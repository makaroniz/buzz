import * as React from "react";
import {
  CircleCheck,
  CircleDashed,
  CircleX,
  GitBranch,
  LoaderCircle,
  MessageSquareText,
} from "lucide-react";

import {
  updateChatWorkAutomation,
  useChatWorkAutomation,
} from "@/features/chats/lib/chatWorkAutomation";
import {
  parseGithubPullRequestRef,
  useGithubCheckSummaryQuery,
  useGithubPullRequestQuery,
} from "@/shared/lib/githubPullRequest";
import { parseSupportedLinkPreview } from "@/shared/lib/linkPreview";
import { cn } from "@/shared/lib/cn";
import { Checkbox } from "@/shared/ui/checkbox";
import { GithubPullRequestCard } from "@/shared/ui/link-preview-attachment";

const CHIP_CLASS =
  "flex items-center gap-1.5 rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-xs";

/**
 * Right-hand work drawer for a chat: branch, live PR card, CI monitor, and
 * automation toggles once the agent has produced a pull request; an empty
 * state before that. When automation is armed, CI failures and new comments
 * prompt the chat's agent automatically (deduped per head sha / comment
 * count).
 */
export function ChatWorkPanel({
  chatId,
  onAutomationPrompt,
  open = true,
  prHref,
}: {
  chatId: string;
  onAutomationPrompt?: (content: string) => void;
  open?: boolean;
  prHref?: string | null;
}) {
  const preview = prHref ? parseSupportedLinkPreview(prHref) : null;
  const ref = prHref ? parseGithubPullRequestRef(prHref) : null;
  const prQuery = useGithubPullRequestQuery(ref);
  const pr = prQuery.data ?? null;
  const checksQuery = useGithubCheckSummaryQuery(ref, pr?.headSha);
  const checks = checksQuery.data ?? null;
  const automation = useChatWorkAutomation(chatId);
  const commentTotal = pr ? pr.comments + pr.reviewComments : 0;

  // Automation: prompt the agent on CI failure / new comments. Watermarks in
  // storage keep this to one nudge per failing sha and per comment increase.
  React.useEffect(() => {
    if (!onAutomationPrompt || !prHref || !pr) {
      return;
    }
    if (
      automation.autoFixCi &&
      checks &&
      checks.failed > 0 &&
      checks.pending === 0 &&
      automation.lastCiNudgeSha !== pr.headSha
    ) {
      updateChatWorkAutomation(chatId, { lastCiNudgeSha: pr.headSha });
      onAutomationPrompt(
        `CI is failing on ${prHref} (${checks.failed} of ${checks.total} checks). Investigate the failures and push fixes until the checks pass.`,
      );
    }
    if (
      automation.addressComments &&
      commentTotal > 0 &&
      (automation.lastCommentNudgeCount ?? 0) < commentTotal
    ) {
      updateChatWorkAutomation(chatId, { lastCommentNudgeCount: commentTotal });
      onAutomationPrompt(
        `There are review comments on ${prHref}. Address each comment and its replies, push any needed changes, reply to the threads, and resolve every conversation that has been addressed.`,
      );
    }
  }, [
    automation,
    chatId,
    checks,
    commentTotal,
    onAutomationPrompt,
    pr,
    prHref,
  ]);

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
          <div className={CHIP_CLASS}>
            <GitBranch className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            {pr?.headRef?.trim() ? (
              <span className="min-w-0 truncate font-mono">
                {pr.headRef.trim()}
              </span>
            ) : (
              <span className="text-muted-foreground">No current branch</span>
            )}
          </div>
          {preview ? (
            <GithubPullRequestCard className="w-full" preview={preview} />
          ) : null}
          {preview ? (
            <div className={CHIP_CLASS} data-testid="chat-ci-monitor">
              <CiStatus checks={checks} />
              <span
                aria-hidden="true"
                className="mx-0.5 text-muted-foreground/50"
              >
                ·
              </span>
              <MessageSquareText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="text-muted-foreground">
                {commentTotal} comment{commentTotal === 1 ? "" : "s"}
              </span>
            </div>
          ) : null}
          {preview ? (
            <div className={cn(CHIP_CLASS, "flex-col items-stretch gap-2.5")}>
              <label
                className="flex cursor-pointer items-center gap-2"
                htmlFor="automation-auto-fix-ci"
              >
                <Checkbox
                  checked={automation.autoFixCi}
                  data-testid="automation-auto-fix-ci"
                  id="automation-auto-fix-ci"
                  onCheckedChange={(checked) =>
                    updateChatWorkAutomation(chatId, {
                      autoFixCi: checked === true,
                    })
                  }
                />
                <span>Auto-fix CI failures</span>
              </label>
              <label
                className="flex cursor-pointer items-center gap-2"
                htmlFor="automation-address-comments"
              >
                <Checkbox
                  checked={automation.addressComments}
                  data-testid="automation-address-comments"
                  id="automation-address-comments"
                  onCheckedChange={(checked) =>
                    updateChatWorkAutomation(chatId, {
                      addressComments: checked === true,
                    })
                  }
                />
                <span>Address comments & resolve</span>
              </label>
            </div>
          ) : null}
        </div>
      </div>
    </aside>
  );
}

function CiStatus({
  checks,
}: {
  checks: {
    total: number;
    pending: number;
    failed: number;
    succeeded: number;
  } | null;
}) {
  if (!checks || checks.total === 0) {
    return (
      <>
        <CircleDashed className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground">No checks</span>
      </>
    );
  }
  if (checks.pending > 0) {
    return (
      <>
        <LoaderCircle className="sprout-arc-spinner h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground">
          CI running ({checks.total - checks.pending}/{checks.total})
        </span>
      </>
    );
  }
  if (checks.failed > 0) {
    return (
      <>
        <CircleX className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-deleted)]" />
        <span className="font-medium text-[color:var(--status-deleted)]">
          CI failing ({checks.failed})
        </span>
      </>
    );
  }
  return (
    <>
      <CircleCheck className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-added)]" />
      <span className="font-medium text-[color:var(--status-added)]">
        CI passing
      </span>
    </>
  );
}
