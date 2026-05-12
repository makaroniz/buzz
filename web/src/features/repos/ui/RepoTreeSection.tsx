import { ChevronRight, FolderClosed, FileText } from "lucide-react";
import { useState } from "react";

import type { TreeEntry } from "../use-git-browse";
import { useTree } from "../use-git-browse";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function sortEntries(entries: TreeEntry[]): TreeEntry[] {
  return [...entries].sort((a, b) => {
    // Directories first, then files
    if (a.type === "tree" && b.type !== "tree") return -1;
    if (a.type !== "tree" && b.type === "tree") return 1;
    return a.name.localeCompare(b.name);
  });
}

function Breadcrumb({
  path,
  onNavigate,
}: {
  path: string;
  onNavigate: (path: string) => void;
}) {
  const segments = path.split("/").filter(Boolean);

  return (
    <div className="mb-3 flex items-center gap-1 text-sm text-muted-foreground">
      <button
        type="button"
        onClick={() => onNavigate("")}
        className="hover:text-foreground"
      >
        root
      </button>
      {segments.map((segment, i) => {
        const segmentPath = segments.slice(0, i + 1).join("/");
        return (
          <span key={segmentPath} className="flex items-center gap-1">
            <ChevronRight className="h-3 w-3" />
            <button
              type="button"
              onClick={() => onNavigate(segmentPath)}
              className="hover:text-foreground"
            >
              {segment}
            </button>
          </span>
        );
      })}
    </div>
  );
}

function TreeRow({
  entry,
  onNavigate,
}: {
  entry: TreeEntry;
  onNavigate: (name: string) => void;
}) {
  const isDir = entry.type === "tree";

  if (isDir) {
    return (
      <button
        type="button"
        className="flex w-full items-center gap-3 py-2 text-left hover:bg-muted/50"
        onClick={() => onNavigate(entry.name)}
      >
        <FolderClosed className="h-4 w-4 shrink-0 text-blue-500" />
        <span className="min-w-0 flex-1 truncate text-sm">{entry.name}</span>
      </button>
    );
  }

  return (
    <div className="flex w-full items-center gap-3 py-2">
      <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate text-sm">{entry.name}</span>
      {entry.size != null && (
        <span className="shrink-0 text-xs text-muted-foreground">
          {formatSize(entry.size)}
        </span>
      )}
    </div>
  );
}

export function RepoTreeSection({
  repoId,
  owner,
  gitRef,
}: {
  repoId: string;
  owner: string;
  gitRef: string;
}) {
  const [currentPath, setCurrentPath] = useState("");
  const {
    data: entries,
    isLoading,
    error,
  } = useTree(repoId, owner, gitRef, currentPath || undefined);

  if (isLoading) return null;
  if (error) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <FolderClosed className="h-4 w-4" />
          Files
        </h2>
        <p className="text-sm text-destructive">Failed to load files</p>
      </div>
    );
  }
  if (!entries || entries.length === 0) return null;

  const sorted = sortEntries(entries);

  function navigateToDir(name: string) {
    const newPath = currentPath ? `${currentPath}/${name}` : name;
    setCurrentPath(newPath);
  }

  return (
    <div className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <FolderClosed className="h-4 w-4" />
        Files
      </h2>
      {currentPath && (
        <Breadcrumb path={currentPath} onNavigate={setCurrentPath} />
      )}
      <div className="divide-y divide-border rounded-md border border-border">
        {sorted.map((entry) => (
          <div key={entry.name} className="px-3">
            <TreeRow entry={entry} onNavigate={navigateToDir} />
          </div>
        ))}
      </div>
    </div>
  );
}
