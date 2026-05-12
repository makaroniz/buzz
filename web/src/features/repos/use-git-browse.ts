import { useQuery } from "@tanstack/react-query";
import { queryEventsHttp } from "@/shared/lib/query-http";
import type { NostrFilter } from "@/shared/lib/nostr-client";
import { getTag } from "./use-repos";

// ── Types matching the relay synthesis structs ─────────────────────────────

export interface TreeEntry {
  name: string;
  type: string;
  mode: string;
  size: number | null;
  sha: string;
}

export interface CommitEntry {
  sha: string;
  author: string;
  email: string;
  timestamp: number;
  parents: string[];
  message: string;
}

export interface BlobMeta {
  size: number;
  binary: boolean;
  url: string;
}

export interface ReadmeData {
  filename: string;
  content: string;
}

// ── Kind constants (ephemeral git browse events) ───────────────────────────

const KIND_GIT_TREE = 20100;
const KIND_GIT_BLOB = 20101;
const KIND_GIT_COMMIT_LOG = 20102;
const KIND_GIT_README = 20103;

// ── Hooks ──────────────────────────────────────────────────────────────────

/**
 * Fetch a directory listing for a repo at a given ref and optional path.
 */
export function useTree(
  repoId: string,
  owner: string,
  gitRef: string,
  path?: string,
) {
  return useQuery({
    queryKey: ["git-tree", owner, repoId, gitRef, path ?? ""],
    queryFn: async (): Promise<TreeEntry[]> => {
      const filter: NostrFilter = {
        kinds: [KIND_GIT_TREE],
        "#d": [`${owner}/${repoId}`],
        "#r": [gitRef],
        ...(path ? { "#f": [path] } : {}),
      };
      const events = await queryEventsHttp(filter);
      if (events.length === 0) return [];
      return JSON.parse(events[0].content) as TreeEntry[];
    },
    enabled: !!repoId && !!owner && !!gitRef,
    staleTime: 30_000,
  });
}

/**
 * Fetch blob metadata (size, binary flag, download URL) for a file.
 */
export function useBlob(
  repoId: string,
  owner: string,
  gitRef: string,
  path: string,
) {
  return useQuery({
    queryKey: ["git-blob", owner, repoId, gitRef, path],
    queryFn: async (): Promise<BlobMeta | null> => {
      const events = await queryEventsHttp({
        kinds: [KIND_GIT_BLOB],
        "#d": [`${owner}/${repoId}`],
        "#r": [gitRef],
        "#f": [path],
      });
      if (events.length === 0) return null;
      const e = events[0];
      return {
        size: Number(getTag(e, "size") ?? "0"),
        binary: getTag(e, "binary") === "true",
        url: getTag(e, "url") ?? "",
      };
    },
    enabled: !!repoId && !!owner && !!gitRef && !!path,
    staleTime: 30_000,
  });
}

/**
 * Fetch the commit log for a repo at a given ref, with optional pagination.
 */
export function useCommits(
  repoId: string,
  owner: string,
  gitRef: string,
  page?: number,
) {
  return useQuery({
    queryKey: ["git-commits", owner, repoId, gitRef, page ?? 0],
    queryFn: async (): Promise<CommitEntry[]> => {
      const filter: NostrFilter = {
        kinds: [KIND_GIT_COMMIT_LOG],
        "#d": [`${owner}/${repoId}`],
        "#r": [gitRef],
        ...(page !== undefined && page > 0 ? { "#n": [String(page)] } : {}),
      };
      const events = await queryEventsHttp(filter);
      if (events.length === 0) return [];
      return JSON.parse(events[0].content) as CommitEntry[];
    },
    enabled: !!repoId && !!owner && !!gitRef,
    staleTime: 30_000,
  });
}

/**
 * Fetch the README for a repo at a given ref.
 */
export function useReadme(repoId: string, owner: string, gitRef: string) {
  return useQuery({
    queryKey: ["git-readme", owner, repoId, gitRef],
    queryFn: async (): Promise<ReadmeData | null> => {
      const events = await queryEventsHttp({
        kinds: [KIND_GIT_README],
        "#d": [`${owner}/${repoId}`],
        "#r": [gitRef],
      });
      if (events.length === 0) return null;
      const e = events[0];
      return {
        filename: getTag(e, "filename") ?? "README",
        content: e.content,
      };
    },
    enabled: !!repoId && !!owner && !!gitRef,
    staleTime: 30_000,
  });
}
