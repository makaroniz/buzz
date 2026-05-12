import { GitCommitHorizontal } from "lucide-react";

import { relativeTime } from "@/shared/lib/relative-time";
import type { CommitEntry } from "../use-git-browse";
import { useCommits } from "../use-git-browse";

function CommitRow({ commit }: { commit: CommitEntry }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <code className="shrink-0 pt-0.5 font-mono text-xs text-muted-foreground">
        {commit.sha.slice(0, 7)}
      </code>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm">{commit.message}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">
          {commit.author} &middot; {relativeTime(commit.timestamp)}
        </p>
      </div>
    </div>
  );
}

export function RepoCommitsSection({
  repoId,
  owner,
  gitRef,
}: {
  repoId: string;
  owner: string;
  gitRef: string;
}) {
  const { data: commits, isLoading, error } = useCommits(repoId, owner, gitRef);

  if (isLoading) return null;

  if (error) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <GitCommitHorizontal className="h-4 w-4" />
          Commits
        </h2>
        <p className="text-sm text-destructive">Failed to load commits</p>
      </div>
    );
  }

  if (!commits || commits.length === 0) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <GitCommitHorizontal className="h-4 w-4" />
          Commits
        </h2>
        <p className="text-sm text-muted-foreground">No commits yet</p>
      </div>
    );
  }

  return (
    <div className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <GitCommitHorizontal className="h-4 w-4" />
        Recent Commits
      </h2>
      <div className="divide-y divide-border rounded-md border border-border">
        {commits.map((commit) => (
          <div key={commit.sha} className="px-3">
            <CommitRow commit={commit} />
          </div>
        ))}
      </div>
    </div>
  );
}
