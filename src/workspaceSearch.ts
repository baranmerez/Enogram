import type { ChangedFile, CommitEntry } from "./types";
import { hasStatus, isConflicted } from "./workingChanges.ts";

export type FileStatusFilter = "all" | "working" | "staged" | "conflicts";

export function filterFiles(files: ChangedFile[], query: string, status: FileStatusFilter = "all"): ChangedFile[] {
  const needle = query.trim().replace(/\\/g, "/").toLowerCase();
  return files.filter((file) => {
    const matchesPath = !needle || [file.path, file.originalPath ?? ""].some((path) => path.replace(/\\/g, "/").toLowerCase().includes(needle));
    const matchesStatus = status === "all"
      || status === "working" && hasStatus(file.worktreeStatus)
      || status === "staged" && hasStatus(file.indexStatus)
      || status === "conflicts" && isConflicted(file);
    return matchesPath && matchesStatus;
  });
}

export function findCommits(commits: CommitEntry[], query: string): CommitEntry[] {
  const needle = query.trim().toLowerCase();
  return needle ? commits.filter((commit) => [commit.subject, commit.author, commit.oid, commit.refs].some((value) => value.toLowerCase().includes(needle))) : [];
}
