export interface ChangedFile {
  path: string;
  originalPath: string | null;
  indexStatus: string;
  worktreeStatus: string;
}

export interface CommitEntry {
  oid: string;
  parents: string[];
  subject: string;
  author: string;
  date: string;
  refs: string;
}

export interface HistorySearchCursor {
  historyToken: string;
  /** Trimmed, Unicode-lowercased query; return this cursor unchanged. */
  query: string;
  offset: number;
}

export interface HistorySearchResult {
  /** Matches full messages, author names/emails, object IDs, and ref names. */
  commits: CommitEntry[];
  /** Cumulative commits examined, not an estimate of total history size. */
  scannedCount: number;
  /** Empty partial batches must continue until complete or explicitly stopped. */
  nextCursor: HistorySearchCursor | null;
  complete: boolean;
}

export interface WorktreeEntry {
  path: string;
  branch: string;
  head: string;
  isBare: boolean;
  isDetached: boolean;
}

export interface BranchEntry {
  name: string;
  head: string;
  current: boolean;
  upstream: string | null;
  upstreamRemote: string | null;
  upstreamBranch: string | null;
  ahead: number | null;
  behind: number | null;
  worktreePath: string | null;
}

export interface RemoteEntry {
  name: string;
  fetchUrl: string;
  pushUrl: string;
  configToken: string;
  configuredFetchUrl: string | null;
  configuredPushUrl: string | null;
  hasExplicitPushUrl: boolean;
  editBlockedReason: string | null;
}

export interface RepositorySnapshot {
  repoPath: string;
  repoName: string;
  mutationToken: string;
  branch: string;
  head: string | null;
  gitVersion: string;
  isUnborn: boolean;
  mergeInProgress: boolean;
  changes: ChangedFile[];
  commits: CommitEntry[];
  worktrees: WorktreeEntry[];
  branches: BranchEntry[];
  remotes: RemoteEntry[];
}

export interface MutationResult {
  snapshot: RepositorySnapshot;
  commitOid: string | null;
  commitTreeChanged: boolean;
}

export interface DiffResult {
  text: string;
  truncated: boolean;
  binary: boolean;
}

export interface PartialDiffResult extends DiffResult {
  selectionToken: string | null;
  /** Zero-based physical line indexes in text.split('\n'); changed lines only. */
  selectableLineIds: number[];
  unsupportedReason: string | null;
}
