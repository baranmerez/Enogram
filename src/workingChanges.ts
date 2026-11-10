import type { ChangedFile } from "./types";

export function hasStatus(status: string): boolean {
  const value = status.trim();
  return value !== "" && value !== "." && value !== "-";
}

export function isConflicted(file: ChangedFile): boolean {
  const indexStatus = file.indexStatus.trim();
  const worktreeStatus = file.worktreeStatus.trim();
  return indexStatus.includes("U") || worktreeStatus.includes("U")
    || (indexStatus === "A" && worktreeStatus === "A")
    || (indexStatus === "D" && worktreeStatus === "D");
}

export function stagePaths(files: ChangedFile[]): string[] {
  return Array.from(new Set(files
    .filter((file) => hasStatus(file.worktreeStatus) && !isConflicted(file))
    .map((file) => file.path)));
}

export function unstagePaths(files: ChangedFile[]): string[] {
  return Array.from(new Set(files
    .filter((file) => hasStatus(file.indexStatus) && !isConflicted(file))
    .map((file) => file.path)));
}
