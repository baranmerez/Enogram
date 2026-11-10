import { isTauri, invoke } from "@tauri-apps/api/core";
import {
  Activity,
  AlertCircle,
  Check,
  ChevronDown,
  ChevronUp,
  CircleDot,
  Clock3,
  FileCode2,
  FileDiff,
  FilePlus2,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  GitFork,
  History,
  LoaderCircle,
  Minus,
  Plus,
  RefreshCw,
  Search,
  SidebarClose,
  SidebarOpen,
  X,
} from "lucide-react";
import {
  type CSSProperties,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { DiffView } from "./DiffView";
import { parseDiffLines } from "./diffLines";
import { HistoryList } from "./HistoryList";
import { RepositoryActions } from "./RepositoryActions";
import {
  repositoryActionCommand,
  repositoryActionSuccess,
  type RepositoryAction,
  type RepositoryActionFeedback,
  type RepositoryActionKind,
} from "./repositoryActionModel";
import { repositoryIdentity } from "./repositoryIdentity";
import { defaultWorkspacePreferences, fitPanelWidths as constrainPanelWidths, loadWorkspacePreferences, saveWorkspacePreferences, type PanelWidths, type WorkspaceDensity } from "./workspacePreferences";
import { filterFiles, findCommits, type FileStatusFilter } from "./workspaceSearch";
import type { ChangedFile, CommitEntry, DiffResult, HistorySearchCursor, HistorySearchResult, MutationResult, PartialDiffResult, RepositorySnapshot } from "./types";
import { hasStatus, isConflicted, stagePaths, unstagePaths } from "./workingChanges";
import "./workspaceControls.css";

const lastRepositoryStorageKey = "enogram.lastRepository.v1";
const DIFF_MIN = 320;
const COLLAPSED_SIDEBAR_WIDTH = 54;
const RESIZE_HANDLE_WIDTH = 6;

type ViewScope = "working" | "history";
type DiffLayer = "worktree" | "index";
type LoadState = "idle" | "loading" | "refreshing" | "ready" | "error";
type DiffState = "idle" | "loading" | "ready" | "error";
type WorkingMutationKind = "stage" | "unstage" | "commit";
type MutationKind = WorkingMutationKind | RepositoryActionKind;

interface MutationFeedback {
  kind: "success" | "error";
  message: string;
  commitOid?: string | null;
  commitTreeChanged?: boolean;
  repositoryAction?: RepositoryAction;
}

interface CommitFilesTarget {
  repositoryKey: string;
  commitOid: string;
}

interface FileSelection {
  path: string | null;
  layer: DiffLayer;
  historyTarget?: CommitFilesTarget;
}

interface CommitFilesState extends CommitFilesTarget {
  files: ChangedFile[];
}

interface PartialDiffPreview {
  snapshot: RepositorySnapshot;
  filePath: string;
  staged: boolean;
  requestId: number;
  result: PartialDiffResult;
}

interface HistorySearchState {
  query: string;
  commits: CommitEntry[];
  scannedCount: number;
  nextCursor: HistorySearchCursor | null;
  pageStart: HistorySearchCursor | null;
  previousStarts: (HistorySearchCursor | null)[];
  complete: boolean;
  loading: boolean;
  stopped: boolean;
  error: string | null;
}

function errorText(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "Beklenmeyen bir hata oluştu.";
}

function workingFileSelection(files: ChangedFile[], current: FileSelection): FileSelection {
  const file = files.find((entry) => entry.path === current.path) ?? files[0];
  if (!file) return { path: null, layer: current.layer };
  const preferredStatus = current.layer === "index" ? file.indexStatus : file.worktreeStatus;
  const layer = hasStatus(preferredStatus)
    ? current.layer
    : hasStatus(file.worktreeStatus) ? "worktree" : "index";
  return { path: file.path, layer };
}

function shortOid(oid: string | null | undefined): string {
  return oid ? oid.slice(0, 8) : "—";
}

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

function displayRepositoryPath(path: string): string {
  const uncPrefix = "\\\\?\\UNC\\";
  if (path.toUpperCase().startsWith(uncPrefix.toUpperCase())) {
    return `\\\\${path.slice(uncPrefix.length)}`;
  }
  const extendedPrefix = "\\\\?\\";
  return path.startsWith(extendedPrefix) ? path.slice(extendedPrefix.length) : path;
}

function loadLastRepositoryPath(): string {
  try {
    const savedPath = window.localStorage.getItem(lastRepositoryStorageKey);
    if (!savedPath || /[\u0000-\u001f\u007f]/.test(savedPath)) return "";
    const path = displayRepositoryPath(savedPath.trim());
    const normalizedPath = path.replace(/\\/g, "/");
    // Relative paths depend on the launch directory, which can change after installation.
    const isAbsolute = /^[a-z]:\//i.test(normalizedPath)
      || /^\/(?!\/)/.test(path)
      || /^\/\/(?![?.](?:\/|$))[^/]+\/[^/]+(?:\/|$)/.test(normalizedPath);
    return isAbsolute ? path : "";
  } catch {
    return "";
  }
}

function rememberRepositoryPath(path: string): void {
  try {
    window.localStorage.setItem(lastRepositoryStorageKey, path);
  } catch {
    // Opening a repository still succeeds when local settings cannot be saved.
  }
}

function initialWorkspacePreferences() {
  try { return loadWorkspacePreferences(window.localStorage); }
  catch { return loadWorkspacePreferences(); }
}

function parentPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/");
}

function formatDate(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value || "Tarih bilinmiyor";
  return new Intl.DateTimeFormat("tr-TR", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(parsed);
}

function fileStatus(file: ChangedFile, layer: DiffLayer): string {
  const value = layer === "index" ? file.indexStatus : file.worktreeStatus;
  if (hasStatus(value)) return value.trim();
  return layer === "index" && hasStatus(file.worktreeStatus) ? "—" : "·";
}

function describeFileStatus(file: ChangedFile): string {
  const parts: string[] = [];
  if (hasStatus(file.indexStatus)) parts.push(`Dizin: ${file.indexStatus.trim()}`);
  if (hasStatus(file.worktreeStatus)) parts.push(`Çalışma ağacı: ${file.worktreeStatus.trim()}`);
  return parts.length ? parts.join(" · ") : "Değişiklik durumu bilinmiyor";
}

function BrowserRequired() {
  return (
    <main className="browser-required">
      <div className="browser-mark"><FolderGit2 size={27} strokeWidth={1.7} /></div>
      <p className="eyebrow">ENOGRAM · YEREL GIT İNCELEYİCİSİ</p>
      <h1>Enogram masaüstünde çalışır</h1>
      <p className="browser-copy">
        Depo verileri, Git komutlarını çalıştıran Tauri masaüstü çekirdeğinden okunur. Bu web önizlemesi
        bilgisayarınızdaki dosyalara erişmez.
      </p>
      <div className="browser-note"><CircleDot size={15} /> Depo verisi gösterilmiyor</div>
    </main>
  );
}

function App() {
  if (!isTauri()) return <BrowserRequired />;
  return <RepositoryApp />;
}

function RepositoryApp() {
  const [initialWorkspace] = useState(initialWorkspacePreferences);
  const [startupPath] = useState(loadLastRepositoryPath);
  const [pathInput, setPathInput] = useState(startupPath);
  const [snapshot, setSnapshot] = useState<RepositorySnapshot | null>(null);
  const [loadState, setLoadState] = useState<LoadState>("idle");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [commitMessage, setCommitMessage] = useState("");
  const [mutationState, setMutationState] = useState<MutationKind | null>(null);
  const [mutationFeedback, setMutationFeedback] = useState<MutationFeedback | null>(null);
  const [activeRepositoryAction, setActiveRepositoryAction] = useState<RepositoryAction | null>(null);
  const [remoteManagerRequest, setRemoteManagerRequest] = useState(0);
  const [viewScope, setViewScope] = useState<ViewScope>("working");
  const [fileSelection, setFileSelection] = useState<FileSelection>({ path: null, layer: "worktree" });
  const { path: selectedPath, layer: diffLayer } = fileSelection;
  const setSelectedPath = useCallback((path: string | null) => {
    setFileSelection((current) => current.path === path ? current : { ...current, path });
  }, []);
  const setDiffLayer = useCallback((layer: DiffLayer) => {
    setFileSelection((current) => current.layer === layer ? current : { ...current, layer });
  }, []);
  const [selectedCommitOid, setSelectedCommitOid] = useState<string | null>(null);
  const [commitFilesState, setCommitFilesState] = useState<CommitFilesState | null>(null);
  const [commitFilesError, setCommitFilesError] = useState<string | null>(null);
  const [commitFilesReload, setCommitFilesReload] = useState(0);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(initialWorkspace.sidebarCollapsed);
  const [density, setDensity] = useState<WorkspaceDensity>(initialWorkspace.density);
  const [preferredPanelWidths, setPreferredPanelWidths] = useState<PanelWidths>(initialWorkspace.panelWidths);
  const [layoutWidth, setLayoutWidth] = useState(Infinity);
  const [layoutSaved, setLayoutSaved] = useState(true);
  const panelWidths = constrainPanelWidths(preferredPanelWidths, layoutWidth, isSidebarCollapsed);
  const { sidebar: sidebarWidth, center: centerWidth } = panelWidths;
  const [fileQuery, setFileQuery] = useState("");
  const [fileStatusFilter, setFileStatusFilter] = useState<FileStatusFilter>("all");
  const [historyQuery, setHistoryQuery] = useState("");
  const [historyRevealOid, setHistoryRevealOid] = useState<string | null>(null);
  const [historySearch, setHistorySearch] = useState<HistorySearchState | null>(null);
  const [diffResult, setDiffResult] = useState<DiffResult | null>(null);
  const [diffState, setDiffState] = useState<DiffState>("idle");
  const [diffError, setDiffError] = useState<string | null>(null);
  const diffLineCount = useMemo(() => parseDiffLines(diffResult?.text ?? "").length, [diffResult?.text]);
  const pathInputRef = useRef<HTMLInputElement>(null);
  const fileSearchRef = useRef<HTMLInputElement>(null);
  const historySearchRef = useRef<HTMLInputElement>(null);
  const historyResultsRef = useRef<HTMLDivElement>(null);
  const worktreeListRef = useRef<HTMLDivElement>(null);
  const revealWorktrees = useRef(false);
  const layoutRef = useRef<HTMLDivElement>(null);
  const repositoryRequest = useRef(0);
  const commitFilesRequest = useRef(0);
  const diffRequest = useRef(0);
  const preparedDiff = useRef<PartialDiffPreview | null>(null);
  const currentPartialPreview = useRef<PartialDiffPreview | null>(null);
  const historyRequest = useRef(0);
  const historySearchInFlight = useRef(0);
  const selectedHistoryCommit = useRef<{ repositoryKey: string; commit: CommitEntry } | null>(null);
  const snapshotRef = useRef<RepositorySnapshot | null>(null);
  const commitDrafts = useRef(new Map<string, string>());
  const repositoryLoadInFlight = useRef(false);
  const mutationInFlight = useRef(false);

  const invalidateHistorySearch = useCallback(() => {
    historyRequest.current += 1;
    historySearchInFlight.current = 0;
    setHistorySearch(null);
    setHistoryRevealOid(null);
  }, []);

  const acceptSnapshot = useCallback((nextSnapshot: RepositorySnapshot, resetSelection = false, preferredCommitOid?: string | null) => {
    snapshotRef.current = nextSnapshot;
    setCommitMessage(commitDrafts.current.get(repositoryIdentity(nextSnapshot.repoPath)) ?? "");
    setSnapshot(nextSnapshot);
    setPathInput(displayRepositoryPath(nextSnapshot.repoPath));
    setLoadError(null);
    setLoadState("ready");
    setSelectedCommitOid((current) => {
      const preferred = preferredCommitOid ?? current;
      const external = selectedHistoryCommit.current;
      if (!resetSelection && !preferredCommitOid && external?.repositoryKey === repositoryIdentity(nextSnapshot.repoPath)
        && external.commit.oid === current) return current;
      return preferred && nextSnapshot.commits.some((commit) => commit.oid === preferred)
        ? preferred
        : nextSnapshot.commits[0]?.oid ?? null;
    });
    // A history selection belongs to its repository and commit, not the working
    // file list. Its refreshed commit files validate the selected path below.
    setFileSelection((current) => !resetSelection && current.historyTarget ? current : workingFileSelection(
      nextSnapshot.changes, resetSelection ? { path: null, layer: "worktree" } : current,
    ));
  }, []);

  const openRepository = useCallback(async (requestedPath: string, refresh = false) => {
    if (mutationInFlight.current) return;
    const normalizedPath = requestedPath.trim();
    if (!normalizedPath) {
      setLoadError("Bir depo yolu girin.");
      setLoadState("error");
      return;
    }

    const requestId = ++repositoryRequest.current;
    invalidateHistorySearch();
    repositoryLoadInFlight.current = true;
    commitFilesRequest.current += 1;
    diffRequest.current += 1;
    setPathInput(normalizedPath);
    setMutationFeedback(null);
    setLoadError(null);
    setDiffResult(null);
    setDiffError(null);
    setDiffState("idle");
    setCommitFilesState(null);
    setCommitFilesError(null);
    if (!refresh) {
      snapshotRef.current = null;
      setSnapshot(null);
      setSelectedPath(null);
      setSelectedCommitOid(null);
      selectedHistoryCommit.current = null;
      setViewScope("working");
      setFileQuery("");
      setFileStatusFilter("all");
      setHistoryQuery("");
      setHistoryRevealOid(null);
    }
    setLoadState(refresh ? "refreshing" : "loading");

    try {
      const nextSnapshot = await invoke<RepositorySnapshot>("inspect_repository", { path: normalizedPath });
      if (requestId !== repositoryRequest.current) return;

      acceptSnapshot(nextSnapshot, !refresh);
      rememberRepositoryPath(nextSnapshot.repoPath);
    } catch (error) {
      if (requestId !== repositoryRequest.current) return;
      snapshotRef.current = null;
      setSnapshot(null);
      setSelectedPath(null);
      setLoadError(errorText(error));
      setLoadState("error");
    } finally {
      if (requestId === repositoryRequest.current) repositoryLoadInFlight.current = false;
    }
  }, [acceptSnapshot, setSelectedPath, invalidateHistorySearch]);

  useEffect(() => {
    if (startupPath) void openRepository(startupPath);
    else pathInputRef.current?.focus();
    return () => {
      repositoryRequest.current += 1;
      commitFilesRequest.current += 1;
      diffRequest.current += 1;
      historyRequest.current += 1;
    };
  }, [openRepository, startupPath]);

  const workingFiles = useMemo(() => filterFiles(snapshot?.changes ?? [], fileQuery, fileStatusFilter), [snapshot, fileQuery, fileStatusFilter]);
  const selectedWorkingFile = workingFiles.find((file) => file.path === selectedPath) ?? null;
  const selectedCommitFiles = snapshot && commitFilesState?.repositoryKey === repositoryIdentity(snapshot.repoPath)
    && commitFilesState.commitOid === selectedCommitOid ? commitFilesState.files : null;
  const historyFiles = useMemo(() => filterFiles(selectedCommitFiles ?? [], fileQuery), [selectedCommitFiles, fileQuery]);
  const selectedHistoryFile = historyFiles.find((file) => file.path === selectedPath) ?? null;
  const visibleFiles = viewScope === "working" ? workingFiles : historyFiles;
  const totalFiles = viewScope === "working" ? snapshot?.changes.length ?? 0 : selectedCommitFiles?.length ?? 0;
  const fileFilterActive = Boolean(fileQuery.trim()) || viewScope === "working" && fileStatusFilter !== "all";
  const activeFile = viewScope === "working" ? selectedWorkingFile : selectedHistoryFile;
  const unstagedCount = snapshot?.changes.filter((file) => hasStatus(file.worktreeStatus)).length ?? 0;
  const conflictCount = snapshot?.changes.filter(isConflicted).length ?? 0;
  const visibleStagePaths = useMemo(() => stagePaths(workingFiles), [workingFiles]);
  const visibleUnstagePaths = useMemo(() => unstagePaths(workingFiles), [workingFiles]);
  const allUnstagePaths = useMemo(() => unstagePaths(snapshot?.changes ?? []), [snapshot]);
  const stagedCount = allUnstagePaths.length;
  const selectedStagePaths = stagePaths(selectedWorkingFile ? [selectedWorkingFile] : []);
  const selectedUnstagePaths = unstagePaths(selectedWorkingFile ? [selectedWorkingFile] : []);
  const isMutationBusy = mutationState !== null;
  const isRepositoryBusy = loadState === "loading" || loadState === "refreshing";
  const targetsLocked = isMutationBusy || isRepositoryBusy;
  const canMutate = Boolean(snapshot?.mutationToken) && loadState === "ready" && !isMutationBusy;
  const partialPreview = viewScope === "working" && diffState === "ready"
    && preparedDiff.current?.snapshot === snapshot && preparedDiff.current.result === diffResult
    && preparedDiff.current.filePath === activeFile?.path
    && preparedDiff.current.staged === (diffLayer === "index")
    && preparedDiff.current.requestId === diffRequest.current ? preparedDiff.current : null;
  currentPartialPreview.current = partialPreview;
  const canCommit = canMutate && (stagedCount > 0 || Boolean(snapshot?.mergeInProgress))
    && conflictCount === 0 && commitMessage.trim().length > 0;
  const selectedCommit = snapshot?.commits.find((commit) => commit.oid === selectedCommitOid)
    ?? (snapshot && selectedHistoryCommit.current?.repositoryKey === repositoryIdentity(snapshot.repoPath)
      && selectedHistoryCommit.current.commit.oid === selectedCommitOid ? selectedHistoryCommit.current.commit : null);
  const historyMatches = useMemo(() => historySearch?.commits ?? findCommits(snapshot?.commits ?? [], historyQuery), [snapshot, historyQuery, historySearch]);
  const matchingOids = useMemo(() => new Set(historyMatches.map((commit) => commit.oid)), [historyMatches]);
  const historyMatchIndex = historyMatches.findIndex((commit) => commit.oid === historyRevealOid);

  useEffect(() => {
    historyResultsRef.current?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: "nearest" });
  }, [historyRevealOid]);

  useEffect(() => {
    if (targetsLocked || !snapshot || viewScope === "history" && !selectedCommitFiles) return;
    setFileSelection((current) => {
      const next = viewScope === "working" ? workingFileSelection(visibleFiles, current)
        : { ...current, path: visibleFiles.some((file) => file.path === current.path) ? current.path : visibleFiles[0]?.path ?? null };
      return current.path === next.path && current.layer === next.layer ? current : next;
    });
  }, [visibleFiles, viewScope, targetsLocked, snapshot, selectedCommitFiles]);

  const executeMutation = async (
    kind: MutationKind,
    currentSnapshot: RepositorySnapshot,
    command: string,
    args: Record<string, unknown>,
    successMessage: string,
    repositoryAction?: RepositoryAction,
  ): Promise<MutationFeedback | null> => {
    // The ref closes the gap before React renders disabled controls; the snapshot
    // identity also rejects an event from the render preceding a completed write.
    if (mutationInFlight.current || repositoryLoadInFlight.current
      || currentSnapshot !== snapshotRef.current || currentSnapshot !== snapshot
      || loadState !== "ready" || !currentSnapshot.mutationToken) return null;

    mutationInFlight.current = true;
    invalidateHistorySearch();
    const requestId = ++repositoryRequest.current;
    commitFilesRequest.current += 1;
    diffRequest.current += 1;
    setMutationState(kind);
    setActiveRepositoryAction(repositoryAction ?? null);
    setMutationFeedback(null);
    setCommitFilesState(null);
    setCommitFilesError(null);
    setDiffResult(null);
    setDiffError(null);
    setDiffState("loading");
    const repoPath = currentSnapshot.repoPath;

    try {
      const result = await invoke<MutationResult>(command, args);
      if (requestId !== repositoryRequest.current) return null;
      if (kind === "commit") commitDrafts.current.delete(repositoryIdentity(repoPath));
      acceptSnapshot(result.snapshot, false, result.commitOid);
      const commitTreeChanged = kind === "commit" && result.commitTreeChanged;
      const feedback: MutationFeedback = {
        kind: "success",
        message: commitTreeChanged
          ? "Commit oluşturuldu; commit sırasında stage edilen içerik değişti. Oluşturulan commit’i kontrol edin."
          : successMessage,
        commitOid: result.commitOid,
        commitTreeChanged,
        repositoryAction,
      };
      setMutationFeedback(feedback);
      return feedback;
    } catch (error) {
      if (requestId !== repositoryRequest.current) return null;
      const message = errorText(error);
      let feedback: MutationFeedback = { kind: "error", message, repositoryAction };
      setMutationFeedback(feedback);
      setLoadState("refreshing");
      // Git can change state before returning an error. Refresh this exact repo,
      // retaining the draft and never automatically retrying the write.
      try {
        const nextSnapshot = await invoke<RepositorySnapshot>("inspect_repository", { path: repoPath });
        if (requestId !== repositoryRequest.current) return null;
        acceptSnapshot(nextSnapshot);
      } catch (refreshError) {
        if (requestId !== repositoryRequest.current) return null;
        snapshotRef.current = null;
        setSnapshot(null);
        setSelectedPath(null);
        setLoadError(errorText(refreshError));
        setLoadState("error");
        feedback = { kind: "error", message: `${message}\nDepo durumu yenilenemedi: ${errorText(refreshError)}`, repositoryAction };
        setMutationFeedback(feedback);
      }
      return feedback;
    } finally {
      mutationInFlight.current = false;
      if (requestId === repositoryRequest.current) {
        setMutationState(null);
        setActiveRepositoryAction(null);
      }
    }
  };

  const runMutation = async (kind: WorkingMutationKind, selectedOnly = false) => {
    const currentSnapshot = snapshotRef.current;
    if (!currentSnapshot) return;
    const candidates = kind === "commit" ? currentSnapshot.changes : filterFiles(currentSnapshot.changes, fileQuery, fileStatusFilter);
    const targetFiles = selectedOnly ? candidates.filter((file) => file.path === selectedPath) : candidates;
    const paths = kind === "stage" ? stagePaths(targetFiles) : unstagePaths(targetFiles);
    if (kind === "commit") {
      if (!commitMessage.trim() || (!paths.length && !currentSnapshot.mergeInProgress)
        || currentSnapshot.changes.some(isConflicted)) return;
    } else if (!paths.length) return;
    const target = { repoPath: currentSnapshot.repoPath, expectedToken: currentSnapshot.mutationToken };
    await executeMutation(
      kind,
      currentSnapshot,
      kind === "commit" ? "commit_index" : kind === "stage" ? "stage_paths" : "unstage_paths",
      kind === "commit" ? { ...target, message: commitMessage } : { ...target, paths },
      kind === "commit" ? "Commit oluşturuldu." : kind === "stage" ? `${paths.length} dosya stage edildi.` : `${paths.length} dosya unstage edildi.`,
    );
  };

  const runPartialSelection = async (selection: { selectionToken: string; lineIds: number[] }): Promise<void> => {
    const preview = partialPreview;
    // Bind retained callbacks to the exact loaded diff, even if the file, layer,
    // or repository changed without changing HEAD or the index token.
    if (!preview || preview !== currentPartialPreview.current
      || preview.requestId !== diffRequest.current || !canMutate
      || preview.result.binary || preview.result.truncated || preview.result.unsupportedReason
      || !preview.result.selectionToken || selection.selectionToken !== preview.result.selectionToken) return;
    const eligible = new Set(preview.result.selectableLineIds);
    if (!selection.lineIds.length || selection.lineIds.some((id) => !Number.isSafeInteger(id) || !eligible.has(id))) return;
    const lineIds = [...new Set(selection.lineIds)].sort((a, b) => a - b);
    await executeMutation(
      preview.staged ? "unstage" : "stage",
      preview.snapshot,
      "apply_partial_selection",
      {
        repoPath: preview.snapshot.repoPath,
        expectedToken: preview.snapshot.mutationToken,
        filePath: preview.filePath,
        staged: preview.staged,
        selectionToken: selection.selectionToken,
        lineIds,
      },
      `${lineIds.length} değişiklik satırı ${preview.staged ? "unstage" : "stage"} edildi.`,
    );
  };

  const runRepositoryAction = async (action: RepositoryAction): Promise<RepositoryActionFeedback | null> => {
    const currentSnapshot = snapshotRef.current;
    if (!currentSnapshot || action.repoPath !== currentSnapshot.repoPath
      || action.expectedToken !== currentSnapshot.mutationToken) return null;
    const { command, args } = repositoryActionCommand(action);
    const feedback = await executeMutation(action.kind, currentSnapshot, command, args, repositoryActionSuccess(action), action);
    return feedback ? { kind: feedback.kind, message: feedback.message, repositoryAction: action } : null;
  };

  useEffect(() => {
    if (!snapshot || viewScope !== "history" || !selectedCommitOid) return;
    const requestId = ++commitFilesRequest.current;
    const repoPath = snapshot.repoPath;
    const target = { repositoryKey: repositoryIdentity(repoPath), commitOid: selectedCommitOid };
    setCommitFilesState(null);
    setCommitFilesError(null);
    setFileSelection((current) => current.historyTarget?.repositoryKey === target.repositoryKey
      && current.historyTarget.commitOid === target.commitOid
      ? current
      : { ...current, path: null, historyTarget: target });
    diffRequest.current += 1;
    setDiffResult(null);
    setDiffState("idle");

    void invoke<ChangedFile[]>("get_commit_files", { repoPath, commitOid: selectedCommitOid })
      .then((files) => {
        if (requestId !== commitFilesRequest.current || !snapshotRef.current
          || repositoryIdentity(snapshotRef.current.repoPath) !== target.repositoryKey) return;
        setCommitFilesState({ ...target, files });
        setFileSelection((current) => {
          if (current.historyTarget?.repositoryKey !== target.repositoryKey
            || current.historyTarget.commitOid !== target.commitOid) return current;
          const path = files.some((file) => file.path === current.path) ? current.path : files[0]?.path ?? null;
          return current.path === path ? current : { ...current, path };
        });
      })
      .catch((error: unknown) => {
        if (requestId !== commitFilesRequest.current) return;
        setCommitFilesError(errorText(error));
      });
  }, [snapshot, viewScope, selectedCommitOid, commitFilesReload]);

  useEffect(() => {
    if (!snapshot || !activeFile) {
      diffRequest.current += 1;
      setDiffResult(null);
      setDiffError(null);
      setDiffState("idle");
      return;
    }

    const requestId = ++diffRequest.current;
    const repoPath = snapshot.repoPath;
    const filePath = activeFile.path;
    const commitOid = viewScope === "history" ? selectedCommitOid : null;
    const staged = viewScope === "working" && diffLayer === "index";
    setDiffResult(null);
    setDiffError(null);
    setDiffState("loading");

    const isWorking = viewScope === "working";
    void invoke<DiffResult | PartialDiffResult>(
      isWorking ? "prepare_partial_diff" : "read_diff",
      isWorking ? { repoPath, filePath, staged, expectedToken: snapshot.mutationToken } : { repoPath, filePath, staged, commitOid },
    )
      .then((result) => {
        if (requestId !== diffRequest.current) return;
        preparedDiff.current = isWorking && "selectableLineIds" in result
          ? { snapshot, filePath, staged, requestId, result } : null;
        setDiffResult(result);
        setDiffState("ready");
      })
      .catch((error: unknown) => {
        if (requestId !== diffRequest.current) return;
        setDiffError(errorText(error));
        setDiffState("error");
      });
  }, [snapshot, activeFile, viewScope, selectedCommitOid, diffLayer]);

  const refreshRepository = useCallback(() => {
    if (mutationInFlight.current || repositoryLoadInFlight.current) return;
    if (snapshot) void openRepository(snapshot.repoPath, true);
    else void openRepository(pathInput);
  }, [openRepository, pathInput, snapshot]);

  const changeScope = useCallback((scope: ViewScope) => {
    if (mutationInFlight.current || repositoryLoadInFlight.current) return;
    if (scope === viewScope) return;
    commitFilesRequest.current += 1;
    diffRequest.current += 1;
    setCommitFilesState(null);
    setCommitFilesError(null);
    setDiffResult(null);
    setDiffError(null);
    setDiffState("idle");
    if (scope === "working") {
      setFileSelection((current) => workingFileSelection(snapshot?.changes ?? [], current));
    }
    setViewScope(scope);
  }, [snapshot, viewScope]);

  useEffect(() => {
    const layout = layoutRef.current;
    if (!layout) return;
    const fitPanels = () => {
      setLayoutWidth(layout.clientWidth);
    };
    const observer = new ResizeObserver(fitPanels);
    observer.observe(layout);
    fitPanels();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    try { setLayoutSaved(saveWorkspacePreferences({ density, sidebarCollapsed: isSidebarCollapsed, panelWidths: preferredPanelWidths }, window.localStorage)); }
    catch { setLayoutSaved(false); }
  }, [density, isSidebarCollapsed, preferredPanelWidths]);

  useEffect(() => {
    if (!isSidebarCollapsed && revealWorktrees.current) {
      revealWorktrees.current = false;
      worktreeListRef.current?.focus();
    }
  }, [isSidebarCollapsed]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !event.ctrlKey || event.altKey || event.metaKey) return;
      // A native modal dialog owns the keyboard until it closes.
      // Suppress workspace shortcuts and their browser defaults underneath it.
      if (document.querySelector("dialog[open]")) {
        if (["r", "l", "f", "1", "2"].includes(event.key.toLowerCase())) event.preventDefault();
        return;
      }
      if (event.key.toLowerCase() === "r") {
        event.preventDefault();
        refreshRepository();
      }
      if (event.key === "1") {
        event.preventDefault();
        changeScope("working");
      }
      if (event.key === "2") {
        event.preventDefault();
        changeScope("history");
      }
      if (event.key.toLowerCase() === "l") {
        event.preventDefault();
        if (mutationInFlight.current || repositoryLoadInFlight.current) return;
        pathInputRef.current?.focus();
        pathInputRef.current?.select();
      }
      if (event.key.toLowerCase() === "f") {
        event.preventDefault();
        if (mutationInFlight.current || repositoryLoadInFlight.current) return;
        const input = event.shiftKey ? historySearchRef.current : fileSearchRef.current;
        input?.focus();
        input?.select();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [changeScope, refreshRepository]);

  const submitPath = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (mutationInFlight.current || repositoryLoadInFlight.current) return;
    void openRepository(pathInput);
  };

  const selectWorkingFile = (file: ChangedFile) => {
    if (mutationInFlight.current || repositoryLoadInFlight.current) return;
    if (selectedPath === file.path) return;
    diffRequest.current += 1;
    setFileSelection({ path: file.path, layer: hasStatus(file.worktreeStatus) ? "worktree" : "index" });
  };

  const selectCommit = (commit: CommitEntry) => {
    if (mutationInFlight.current || repositoryLoadInFlight.current || !snapshot || snapshot !== snapshotRef.current) return;
    if (viewScope === "history" && selectedCommitOid === commit.oid) return;
    selectedHistoryCommit.current = { repositoryKey: repositoryIdentity(snapshot.repoPath), commit };
    diffRequest.current += 1;
    commitFilesRequest.current += 1;
    changeScope("history");
    setSelectedCommitOid(commit.oid);
    setSelectedPath(null);
    setCommitFilesState(null);
    setDiffResult(null);
    setDiffState("idle");
  };

  const selectHistoryFile = (file: ChangedFile) => {
    if (mutationInFlight.current || repositoryLoadInFlight.current) return;
    if (selectedPath === file.path) return;
    diffRequest.current += 1;
    setSelectedPath(file.path);
  };

  const revealHistoryMatch = (direction: number) => {
    if (mutationInFlight.current || repositoryLoadInFlight.current || !historySearch || historySearch.error || !historyMatches.length) return;
    const next = historyMatchIndex < 0 ? direction > 0 ? 0 : historyMatches.length - 1
      : (historyMatchIndex + direction + historyMatches.length) % historyMatches.length;
    const commit = historyMatches[next];
    setHistoryRevealOid(commit.oid);
    selectCommit(commit);
  };

  const runHistorySearch = async (mode: "start" | "next" | "previous" | "resume" = "start") => {
    const target = snapshotRef.current;
    const query = historyQuery.trim();
    if (!target || target !== snapshot || !query || targetsLocked || mutationInFlight.current
      || repositoryLoadInFlight.current || historySearchInFlight.current) return;
    if (mode === "resume" && historySearch?.stopped && !historySearch.nextCursor) mode = "start";
    if (mode !== "start" && (!historySearch || historySearch.query !== query || historySearch.error)) return;
    if ((mode === "next" || mode === "resume") && !historySearch?.nextCursor) return;
    if (mode === "previous" && !historySearch?.previousStarts.length) return;
    const requestId = ++historyRequest.current;
    historySearchInFlight.current = requestId;
    const previousStarts = mode === "start" ? [] : [...historySearch!.previousStarts];
    let cursor = mode === "next" || mode === "resume" ? historySearch!.nextCursor
      : mode === "previous" ? previousStarts.pop()! : null;
    if (mode === "next") previousStarts.push(historySearch!.pageStart);
    const pageStart = mode === "resume" ? historySearch!.pageStart : cursor;
    let commits = mode === "resume" ? [...historySearch!.commits] : [];
    let current: HistorySearchState = {
      query, commits, scannedCount: cursor?.offset ?? 0, nextCursor: cursor, pageStart, previousStarts,
      complete: false, loading: true, stopped: false, error: null,
    };
    setHistorySearch(current);
    if (mode !== "resume") setHistoryRevealOid(null);
    const isCurrent = () => requestId === historyRequest.current && target === snapshotRef.current
      && !mutationInFlight.current && !repositoryLoadInFlight.current;
    try {
      while (isCurrent()) {
        const result = await invoke<HistorySearchResult>("search_repository_history", { repoPath: target.repoPath, query, cursor });
        if (!isCurrent()) return;
        commits = [...commits, ...result.commits];
        current = { ...current, commits, scannedCount: result.scannedCount, nextCursor: result.nextCursor,
          complete: result.complete, loading: !result.complete && commits.length < 100 };
        setHistorySearch(current);
        if (!current.loading) break;
        cursor = result.nextCursor;
      }
    } catch (error) {
      if (isCurrent()) setHistorySearch({ ...current, loading: false, error: errorText(error) });
    } finally {
      if (historySearchInFlight.current === requestId) historySearchInFlight.current = 0;
    }
  };

  const stopHistorySearch = () => {
    historyRequest.current += 1;
    historySearchInFlight.current = 0;
    setHistorySearch((current) => current ? { ...current, loading: false, stopped: true } : null);
  };

  const resetWorkspaceLayout = () => {
    setDensity(defaultWorkspacePreferences.density);
    setPreferredPanelWidths(defaultWorkspacePreferences.panelWidths);
    setIsSidebarCollapsed(defaultWorkspacePreferences.sidebarCollapsed);
  };

  const startResize = (kind: "sidebar" | "center", event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const layout = layoutRef.current;
    if (!layout) return;
    if (kind === "sidebar" && isSidebarCollapsed) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const bounds = layout.getBoundingClientRect();
    const pointerMove = (moveEvent: PointerEvent) => {
      setPreferredPanelWidths((current) => {
        const fitted = constrainPanelWidths(current, layout.clientWidth, isSidebarCollapsed);
        const visibleSidebar = isSidebarCollapsed ? COLLAPSED_SIDEBAR_WIDTH : fitted.sidebar;
        const nextWidth = moveEvent.clientX - bounds.left - (kind === "center" ? visibleSidebar + RESIZE_HANDLE_WIDTH : 0);
        const next = constrainPanelWidths({ ...fitted, [kind]: nextWidth }, layout.clientWidth, isSidebarCollapsed);
        return { ...current, [kind]: next[kind] };
      });
    };
    const pointerUp = () => {
      window.removeEventListener("pointermove", pointerMove);
      window.removeEventListener("pointerup", pointerUp);
      window.removeEventListener("pointercancel", pointerUp);
    };
    window.addEventListener("pointermove", pointerMove);
    window.addEventListener("pointerup", pointerUp, { once: true });
    window.addEventListener("pointercancel", pointerUp, { once: true });
  };

  const keyboardResize = (kind: "sidebar" | "center", event: ReactKeyboardEvent<HTMLDivElement>) => {
    const delta = event.key === "ArrowRight" ? 16 : event.key === "ArrowLeft" ? -16 : 0;
    if (!delta) return;
    event.preventDefault();
    const layout = layoutRef.current;
    if (!layout || (kind === "sidebar" && isSidebarCollapsed)) return;
    setPreferredPanelWidths((current) => {
      const fitted = constrainPanelWidths(current, layout.clientWidth, isSidebarCollapsed);
      const next = constrainPanelWidths({ ...fitted, [kind]: fitted[kind] + delta }, layout.clientWidth, isSidebarCollapsed);
      return { ...current, [kind]: next[kind] };
    });
  };

  const gridStyle = {
    "--sidebar-width": `${isSidebarCollapsed ? COLLAPSED_SIDEBAR_WIDTH : sidebarWidth}px`,
    "--center-width": `${centerWidth}px`,
    "--diff-min-width": `${DIFF_MIN}px`,
    "--resize-handle-width": `${RESIZE_HANDLE_WIDTH}px`,
  } as CSSProperties;

  return (
    <div className="app-shell" data-density={density}>
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-icon"><GitFork size={17} strokeWidth={2.2} /></div>
          <span className="brand-name">enogram</span>
          <span className="brand-version">0.1</span>
        </div>

        <form className="repo-open-form" onSubmit={submitPath}>
          <FolderGit2 className="repo-path-icon" size={16} />
          <input
            ref={pathInputRef}
            aria-label="Depo yolu"
            spellCheck={false}
            disabled={targetsLocked}
            value={pathInput}
            placeholder="Yerel Git deposunun yolunu girin"
            onChange={(event) => {
              if (!mutationInFlight.current && !repositoryLoadInFlight.current) setPathInput(event.target.value);
            }}
            title="Açılacak yerel depo klasörünün yolu"
          />
          <button className="open-button" type="submit" disabled={targetsLocked}>
            {loadState === "loading" ? <LoaderCircle className="spin" size={14} /> : <ChevronDown size={14} />}
            <span>Aç</span>
          </button>
        </form>

        <div className="topbar-separator" />
        <RepositoryActions
          snapshot={snapshot}
          targetsLocked={targetsLocked}
          canMutate={canMutate}
          refreshing={loadState === "refreshing"}
          activeAction={activeRepositoryAction}
          feedback={mutationFeedback?.repositoryAction ? { kind: mutationFeedback.kind, message: mutationFeedback.message, repositoryAction: mutationFeedback.repositoryAction } : null}
          onExecute={runRepositoryAction}
          onDismissFeedback={() => setMutationFeedback(null)}
          onRefresh={refreshRepository}
          remoteManagerRequest={remoteManagerRequest}
          density={density}
          onDensityChange={setDensity}
          onResetLayout={resetWorkspaceLayout}
        />
      </header>

      <div className="workspace" ref={layoutRef} style={gridStyle}>
        <aside className={`sidebar ${isSidebarCollapsed ? "collapsed" : ""}`}>
          <div className="sidebar-heading">
            {!isSidebarCollapsed && <span>GEZİNME</span>}
            <button
              className="subtle-icon-button"
              type="button"
              onClick={() => setIsSidebarCollapsed((collapsed) => !collapsed)}
              title={isSidebarCollapsed ? "Kenar çubuğunu aç" : "Kenar çubuğunu daralt"}
              aria-label={isSidebarCollapsed ? "Kenar çubuğunu aç" : "Kenar çubuğunu daralt"}
            >
              {isSidebarCollapsed ? <SidebarOpen size={15} /> : <SidebarClose size={15} />}
            </button>
          </div>

          {!isSidebarCollapsed ? (
            <>
              <div className="sidebar-section-label"><span>YEREL DEPO</span><span className="section-count">{snapshot ? "1" : "0"}</span></div>
              <div className={`repo-tree-item ${snapshot ? "selected" : ""}`} title={snapshot?.repoPath ?? pathInput}>
                <FolderGit2 size={16} />
                <span className="tree-primary">{snapshot?.repoName || (isRepositoryBusy ? "Depo okunuyor…" : "Depo açık değil")}</span>
                {snapshot?.branch && <span className="tree-branch">{snapshot.branch}</span>}
              </div>

              <div className="sidebar-section-label worktree-label">
                <span>ÇALIŞMA AĞAÇLARI</span>
                {snapshot && <span className="section-count">{snapshot.worktrees.length}</span>}
              </div>
              {snapshot?.worktrees.length ? (
                <div className="worktree-list" ref={worktreeListRef} tabIndex={-1} aria-label="Çalışma ağaçları">
                  {snapshot.worktrees.map((worktree) => {
                    const isCurrent = repositoryIdentity(worktree.path) === repositoryIdentity(snapshot.repoPath);
                    return (
                      <button className={`worktree-item ${isCurrent ? "current" : ""}`} type="button" key={worktree.path} title={worktree.isBare ? `${worktree.path} — Çalışma dizini yok` : worktree.path}
                        aria-label={`Çalışma ağacını aç: ${worktree.path}`} aria-current={isCurrent ? "true" : undefined}
                        disabled={targetsLocked || worktree.isBare || isCurrent}
                        onClick={() => { if (!mutationInFlight.current && !repositoryLoadInFlight.current && !worktree.isBare && !isCurrent) void openRepository(worktree.path); }}>
                        <GitBranch size={14} />
                        <div className="worktree-copy">
                          <span>{worktree.branch || (worktree.isDetached ? "Ayrık HEAD" : "Dal yok")}</span>
                          <small>{basename(worktree.path)}</small>
                        </div>
                        {isCurrent && <span className="current-indicator" aria-label="Açık depo"><Check size={11} /></span>}
                      </button>
                    );
                  })}
                </div>
              ) : snapshot ? (
                <div className="sidebar-empty">Çalışma ağacı bilgisi yok</div>
              ) : null}

              <div className="sidebar-bottom">
                <div className="connection-card">
                  <div className="connection-icon"><Activity size={15} /></div>
                  <div>
                    <strong>Yerel Git</strong>
                    <span>{snapshot?.gitVersion || "Sürüm bilgisi bekleniyor"}</span>
                  </div>
                  <span className={`connection-dot ${snapshot ? "online" : "offline"}`} />
                </div>
                <button className="remote-sidebar-action" type="button" aria-label="Uzak depoları yönet" disabled={!canMutate || targetsLocked} onClick={() => setRemoteManagerRequest((request) => request + 1)}>
                  <span><FolderGit2 size={14} /> Uzak depolar</span><span>{snapshot ? snapshot.remotes.length ? `${snapshot.remotes.length} tanımlı` : "Ekle" : "Depo bekleniyor"}</span>
                </button>
                <div className="future-card" title="Yerel MCP erişimi Codex veya Claude ayarlarından yönetilir. Bu pencere istemci bağlantı durumunu izlemez.">
                  <div><span className="future-bullet" /> Codex / Claude</div><span>Yerel MCP</span>
                </div>
                {snapshot && !snapshot.remotes.length && <p className="remote-setup-hint">Fetch, Pull ve Push için Uzak depolar bölümünden bir adres ekleyin.</p>}
              </div>
            </>
          ) : (
            <div className="collapsed-sidebar-icons">
              <button type="button" title={snapshot?.repoName || "Yerel depo"} aria-label="Depo yoluna git" disabled={targetsLocked} onClick={() => { if (!mutationInFlight.current && !repositoryLoadInFlight.current) { pathInputRef.current?.focus(); pathInputRef.current?.select(); } }}><FolderGit2 size={18} /></button>
              <button type="button" title="Çalışma ağaçları" aria-label="Çalışma ağaçlarını göster" onClick={() => { revealWorktrees.current = true; setIsSidebarCollapsed(false); }}><GitBranch size={18} /></button>
              <div className={`connection-dot ${snapshot ? "online" : "offline"}`} title="Yerel Git" />
            </div>
          )}
        </aside>

        <div
          className="resize-handle"
          role="separator"
          aria-label="Kenar çubuğu genişliği"
          aria-orientation="vertical"
          tabIndex={0}
          onPointerDown={(event) => startResize("sidebar", event)}
          onKeyDown={(event) => keyboardResize("sidebar", event)}
        />

        <main className={`center-panel ${viewScope === "working" ? "working-scope" : ""} ${snapshot?.isUnborn && snapshot.commits.length === 0 && viewScope === "working" ? "unborn-working" : ""}`}>
          <div className="panel-toolbar history-toolbar">
            <div className="scope-tabs" role="tablist" aria-label="İçerik kapsamı">
              <button
                className={viewScope === "working" ? "active" : ""}
                type="button"
                role="tab"
                aria-selected={viewScope === "working"}
                disabled={targetsLocked}
                onClick={() => changeScope("working")}
              >
                <FileDiff size={14} /> Çalışma <span className="tab-count">{snapshot?.changes.length ?? 0}</span>
              </button>
              <button
                className={viewScope === "history" ? "active" : ""}
                type="button"
                role="tab"
                aria-selected={viewScope === "history"}
                disabled={targetsLocked}
                onClick={() => changeScope("history")}
              >
                <History size={14} /> Geçmiş
              </button>
            </div>
            <button className="toolbar-icon" type="button" onClick={refreshRepository} disabled={targetsLocked} aria-label="Geçmişi ve durumu yenile" title="Geçmişi ve durumu yenile">
              <RefreshCw size={14} />
            </button>
          </div>

          <section className="history-section" aria-label="Commit geçmişi">
            <div className="section-title-row history-search-bar" role="search" aria-label="Tüm geçmişte arama">
              <History size={14} aria-hidden="true" />
              <input ref={historySearchRef} aria-label="Commit ara" title="Tüm yerel geçmişte mesaj (başlık ve gövde), yazar/e-posta, SHA veya referans ara. Enter ile ara; Ctrl+Shift+F ile odaklan."
                placeholder="Tüm geçmişte ara…" value={historyQuery} disabled={!snapshot || targetsLocked}
                onChange={(event) => { if (!mutationInFlight.current && !repositoryLoadInFlight.current) { invalidateHistorySearch(); setHistoryQuery(event.target.value); } }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    if (historySearch && !historySearch.loading && !historySearch.error && historyMatches.length) revealHistoryMatch(event.shiftKey ? -1 : 1);
                    else void runHistorySearch();
                  }
                  if (event.key === "Escape") { event.preventDefault(); invalidateHistorySearch(); setHistoryQuery(""); }
                }} />
              <button type="button" className="search-icon" aria-label="Tüm geçmişte ara" title="Tüm yerel geçmişte ara" disabled={!snapshot || targetsLocked || !historyQuery.trim() || Boolean(historySearch?.loading)} onClick={() => void runHistorySearch()}><Search size={14} /></button>
              {historyQuery.trim() ? <>
                {historySearch && <span className="search-count" aria-label={`${historyMatches.length} sonuç bu sayfada`}>{historyMatchIndex + 1}/{historyMatches.length}</span>}
                <button type="button" className="search-icon" title="Önceki eşleşme (Shift+Enter)" aria-label="Önceki commit eşleşmesi" disabled={targetsLocked || !historySearch || Boolean(historySearch.error) || !historyMatches.length} onClick={() => revealHistoryMatch(-1)}><ChevronUp size={14} /></button>
                <button type="button" className="search-icon" title="Sonraki eşleşme (Enter)" aria-label="Sonraki commit eşleşmesi" disabled={targetsLocked || !historySearch || Boolean(historySearch.error) || !historyMatches.length} onClick={() => revealHistoryMatch(1)}><ChevronDown size={14} /></button>
                <button type="button" className="search-icon" aria-label="Commit aramasını temizle" title="Aramayı temizle ve grafiğe dön" disabled={targetsLocked} onClick={() => { invalidateHistorySearch(); setHistoryQuery(""); }}><X size={13} /></button>
              </> : <span className="history-count" title="Grafikte yüklenen son commit sayısı">{snapshot?.commits.length ?? 0}</span>}
            </div>
            {historySearch ? <>
              <div className="history-search-progress" role="status">
                <span>{historySearch.error ? "Arama tamamlanamadı" : historySearch.loading ? "Taranıyor" : historySearch.stopped ? "Durduruldu" : historySearch.complete ? "Tarama tamamlandı" : "Diğer sonuçlar var"} · {historySearch.scannedCount} commit</span>
                {historySearch.loading ? <button type="button" onClick={stopHistorySearch}>Durdur</button> : <>
                  <button type="button" aria-label="Önceki sonuç sayfası" title="Önceki sonuç sayfası" disabled={targetsLocked || Boolean(historySearch.error) || !historySearch.previousStarts.length} onClick={() => void runHistorySearch("previous")}><ChevronUp size={13} /></button>
                  <button type="button" aria-label={historySearch.stopped ? "Aramayı sürdür" : "Sonraki sonuç sayfası"} title={historySearch.stopped ? "Aramayı sürdür" : "Sonraki sonuç sayfası"} disabled={targetsLocked || Boolean(historySearch.error) || !historySearch.nextCursor && !historySearch.stopped} onClick={() => void runHistorySearch(historySearch.stopped ? "resume" : "next")}><ChevronDown size={13} /></button>
                </>}
              </div>
              <div className="history-search-results" ref={historyResultsRef} role="region" aria-label="Tüm geçmiş arama sonuçları">
                {historySearch.error && <div className="history-search-message error-message" role="alert">{historySearch.error}<button type="button" disabled={targetsLocked} onClick={() => void runHistorySearch()}>Yeniden ara</button></div>}
                {!historySearch.commits.length && !historySearch.error && <div className="history-search-message">
                  {historySearch.loading ? <><LoaderCircle className="spin" size={14} /> Geçmiş taranıyor…</> : historySearch.complete ? "Tüm yerel geçmiş tarandı; eşleşen commit bulunamadı." : "Arama durduruldu. Devam edebilir veya yeniden arayabilirsiniz."}
                </div>}
                {historySearch.commits.map((commit) => <button key={commit.oid} type="button" className="history-search-result" disabled={targetsLocked || Boolean(historySearch.error)}
                  aria-pressed={viewScope === "history" && selectedCommitOid === commit.oid} aria-current={historyRevealOid === commit.oid ? "true" : undefined}
                  title={`${commit.subject}\n${commit.oid}\n${commit.author} · ${formatDate(commit.date)}\n${commit.refs}`}
                  onClick={() => { if (!targetsLocked) { setHistoryRevealOid(commit.oid); selectCommit(commit); } }}>
                  <span className="history-result-subject">{commit.subject || "(Başlıksız commit)"}</span>
                  <span className="history-result-meta"><code>{shortOid(commit.oid)}</code><span>{commit.author}</span><span>{formatDate(commit.date)}</span></span>
                  {commit.refs && <span className="history-result-refs">{commit.refs}</span>}
                </button>)}
              </div>
            </> : snapshot && snapshot.commits.length > 0 ? (
              <HistoryList
                commits={snapshot.commits}
                selectedOid={viewScope === "history" ? selectedCommitOid : null}
                onSelect={selectCommit}
                disabled={targetsLocked}
                density={density}
                matchingOids={matchingOids}
                revealOid={historyRevealOid}
              />
            ) : (
              <div className="commit-list">
                {loadState === "idle" && (
                  <div className="history-empty"><FolderGit2 size={20} /><span>Başlamak için bir depo açın.</span></div>
                )}
                {loadState === "loading" && <LoadingRows />}
                {snapshot?.isUnborn && loadState === "ready" && (
                  <div className="history-empty"><GitCommitHorizontal size={20} /><span>Henüz ilk commit yok.</span></div>
                )}
                {snapshot && !snapshot.isUnborn && loadState === "ready" && (
                  <div className="history-empty"><History size={18} /><span>Geçmiş girdisi bulunamadı.</span></div>
                )}
              </div>
            )}
            {loadState === "error" && <InlineError message={loadError ?? "Depo okunamadı."} onRetry={refreshRepository} />}
          </section>

          <section className="files-section" aria-label={viewScope === "working" ? "Çalışma ağacı dosyaları" : "Commit dosyaları"}>
            <div className="files-section-header">
              <div className="files-heading">
                <span>{viewScope === "working" ? "DEĞİŞİKLİKLER" : "COMMIT DOSYALARI"}</span>
                <span className="section-count" title="Görünen / toplam dosya">{fileFilterActive ? `${visibleFiles.length}/${totalFiles}` : totalFiles}</span>
              </div>
              {viewScope === "working" ? (
                <div className="change-summary" title="Bir dosya hem stage edilmiş hem çalışma ağacında değişmiş olabilir.">
                  <span className="index-count" aria-label={`${stagedCount} stage edilmiş dosya`}><span className="mini-mark">S</span>Stage {stagedCount}</span>
                  <span className="worktree-count" aria-label={`${unstagedCount} çalışma ağacı dosyası`}><span className="mini-mark">W</span>Çalışma {unstagedCount}</span>
                </div>
              ) : selectedCommit && <span className="commit-file-oid">{shortOid(selectedCommit.oid)}</span>}
            </div>
            <div className="file-filter-bar" role="search" aria-label="Dosya filtreleri">
              <label className="file-search"><Search size={13} aria-hidden="true" /><input ref={fileSearchRef} aria-label="Dosya ara" title="Dosya adı veya yol (Ctrl+F)" placeholder="Dosya veya yol…" value={fileQuery} disabled={!snapshot || targetsLocked}
                onChange={(event) => { if (!mutationInFlight.current && !repositoryLoadInFlight.current) setFileQuery(event.target.value); }}
                onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); setFileQuery(""); setFileStatusFilter("all"); } }} /></label>
              {viewScope === "working" && <select aria-label="Dosya durumu" value={fileStatusFilter} disabled={!snapshot || targetsLocked}
                onChange={(event) => { if (!mutationInFlight.current && !repositoryLoadInFlight.current) setFileStatusFilter(event.target.value as FileStatusFilter); }}>
                <option value="all">Tümü</option><option value="working">Çalışma (W)</option><option value="staged">Stage (S)</option><option value="conflicts">Çakışmalar</option>
              </select>}
              {fileFilterActive && <button type="button" className="search-icon" aria-label="Dosya filtrelerini temizle" disabled={targetsLocked} onClick={() => { setFileQuery(""); setFileStatusFilter("all"); }}><X size={13} /></button>}
            </div>
            {viewScope === "working" && (
              <div className="bulk-actions" role="group" aria-label={fileFilterActive ? "Görünen dosyalar için stage işlemleri" : "Tüm dosyalar için stage işlemleri"}>
                <button
                  className="change-action"
                  type="button"
                  disabled={!canMutate || visibleStagePaths.length === 0}
                  onClick={() => void runMutation("stage")}
                  title={`${visibleStagePaths.length} görünen uygun dosyayı stage et${conflictCount ? "; çakışmalı dosyalar hariç" : ""}`}
                ><Plus size={13} />{fileFilterActive ? "Görünenleri stage et" : "Tümünü stage et"}<span className="action-count">{visibleStagePaths.length}</span></button>
                <button
                  className="change-action"
                  type="button"
                  disabled={!canMutate || visibleUnstagePaths.length === 0}
                  onClick={() => void runMutation("unstage")}
                  title={`${visibleUnstagePaths.length} görünen uygun dosyayı unstage et${conflictCount ? "; çakışmalı dosyalar hariç" : ""}`}
                ><Minus size={13} />{fileFilterActive ? "Görünenleri unstage et" : "Tümünü unstage et"}<span className="action-count">{visibleUnstagePaths.length}</span></button>
              </div>
            )}
            {viewScope === "history" && (
              <div className="selected-commit-banner">
                {selectedCommit ? (
                  <><GitCommitHorizontal size={14} /><span>{selectedCommit.subject}</span></>
                ) : <span>Dosyalarını incelemek için geçmişten bir commit seçin.</span>}
              </div>
            )}
            <div className="file-list-header">
              <span>Durum</span><span>Dosya</span><span>Yol</span>
            </div>
            <div className="file-list">
              {visibleFiles.map((file) => (
                <FileRow
                  key={`${file.path}-${file.originalPath ?? ""}`}
                  file={file}
                  scope={viewScope}
                  layer={diffLayer}
                  selected={selectedPath === file.path}
                  disabled={targetsLocked}
                  onSelect={() => viewScope === "working" ? selectWorkingFile(file) : selectHistoryFile(file)}
                />
              ))}
              {fileFilterActive && totalFiles > 0 && visibleFiles.length === 0 && <div className="file-empty" role="status"><Search size={16} /><span>Bu filtreyle eşleşen dosya yok.</span></div>}
              {viewScope === "working" && snapshot && totalFiles === 0 && loadState === "ready" && (
                <div className="file-empty"><Check size={16} /><span>Çalışma ağacı temiz.</span></div>
              )}
              {viewScope === "working" && !snapshot && loadState === "loading" && <div className="file-placeholder">Durum okunuyor…</div>}
              {viewScope === "history" && !selectedCommit && (
                <div className="file-empty"><History size={17} /><span>Commit seçilmedi.</span></div>
              )}
              {viewScope === "history" && selectedCommit && !selectedCommitFiles && !commitFilesError && (
                <div className="file-placeholder"><LoaderCircle className="spin" size={14} /> Commit dosyaları okunuyor…</div>
              )}
              {viewScope === "history" && selectedCommitFiles && selectedCommitFiles.length === 0 && (
                <div className="file-empty"><Check size={16} /><span>Bu commit'te dosya değişikliği yok.</span></div>
              )}
              {viewScope === "history" && commitFilesError && <InlineError message={commitFilesError} onRetry={() => setCommitFilesReload((reload) => reload + 1)} />}
              {loadState === "error" && viewScope === "working" && <div className="file-placeholder">Depo açılamadı.</div>}
            </div>
            <div className="files-footer">
              {viewScope === "working" ? (
                <>
                  <span><span className="footer-state staged-state" /> Stage edilmiş {stagedCount}</span>
                  <span><span className="footer-state working-state" /> Çalışma ağacında {unstagedCount}</span>
                </>
              ) : <span>{selectedCommit ? `${shortOid(selectedCommit.oid)} · ${formatDate(selectedCommit.date)}` : "Geçmiş kapsamı"}</span>}
            </div>
          </section>

          {viewScope === "working" && (
            <form className="commit-composer" onSubmit={(event) => { event.preventDefault(); void runMutation("commit"); }}>
              <div className="commit-composer-heading">
                <label htmlFor="commit-message">Commit mesajı</label>
                <span>{stagedCount} dosya stage edilmiş</span>
              </div>
              <textarea
                id="commit-message"
                rows={2}
                value={commitMessage}
                disabled={targetsLocked || !snapshot}
                onChange={(event) => {
                  if (mutationInFlight.current || repositoryLoadInFlight.current
                    || !snapshot || snapshot !== snapshotRef.current) return;
                  const message = event.target.value;
                  commitDrafts.current.set(repositoryIdentity(snapshot.repoPath), message);
                  setCommitMessage(message);
                }}
                placeholder="Değişikliği kısaca açıklayın…"
                aria-describedby={snapshot?.mergeInProgress ? "commit-scope-hint commit-merge-hint" : "commit-scope-hint"}
              />
              <div className="commit-submit-row">
                <span id="commit-scope-hint">{fileFilterActive ? "Filtre dışındakiler dahil tüm stage edilen dosyalar commit edilir." : "Yalnızca stage edilen dosyalar commit edilir."}</span>
                <button className="commit-button" type="submit" disabled={!canCommit}>
                  {mutationState === "commit" ? <LoaderCircle className="spin" size={14} /> : <GitCommitHorizontal size={14} />}
                  {mutationState === "commit" ? "Commit ediliyor…" : "Commit"}
                </button>
              </div>
              {snapshot?.mergeInProgress && (
                <div className="mutation-progress" id="commit-merge-hint" role="status"><GitFork size={14} /><span>Merge bekliyor. Stage edilmiş değişiklik olmasa da commit ile tamamlayabilirsiniz.</span></div>
              )}
              {conflictCount > 0 && (
                <div className="commit-conflict" role="status"><AlertCircle size={14} /><span>{conflictCount} çakışmalı dosya var. Commit için çakışmaları çözün. Toplu işlemler bu dosyaları atlar.</span></div>
              )}
              {isMutationBusy && !activeRepositoryAction && (
                <div className="mutation-progress" role="status"><LoaderCircle className="spin" size={13} />{loadState === "refreshing" ? "Depo durumu yeniden okunuyor…" : mutationState === "commit" ? "Commit oluşturuluyor…" : mutationState === "stage" ? "Dosyalar stage ediliyor…" : "Dosyalar unstage ediliyor…"}</div>
              )}
              {mutationFeedback && !mutationFeedback.repositoryAction && (
                <div className={`mutation-feedback ${mutationFeedback.commitTreeChanged ? "warning" : mutationFeedback.kind}`} role={mutationFeedback.kind === "error" || mutationFeedback.commitTreeChanged ? "alert" : "status"}>
                  {mutationFeedback.kind === "error" || mutationFeedback.commitTreeChanged ? <AlertCircle size={14} /> : <Check size={14} />}
                  <span>{mutationFeedback.message}{mutationFeedback.commitOid && <> <code title={mutationFeedback.commitOid}>{shortOid(mutationFeedback.commitOid)}</code></>}</span>
                </div>
              )}
            </form>
          )}
        </main>

        <div
          className="resize-handle"
          role="separator"
          aria-label="Geçmiş ve fark paneli genişliği"
          aria-orientation="vertical"
          tabIndex={0}
          onPointerDown={(event) => startResize("center", event)}
          onKeyDown={(event) => keyboardResize("center", event)}
        />

        <section className={`diff-panel ${viewScope === "working" ? "working-diff" : ""}`} aria-label="Seçili dosya farkı">
          <div className="diff-panel-header">
            <div className="diff-context">
              <span className="diff-context-icon"><FileCode2 size={14} /></span>
              <div className="diff-heading-text">
                <div className="diff-source">
                  {viewScope === "working"
                    ? (diffLayer === "index" ? "DİZİN · HEAD İLE KARŞILAŞTIRMA" : "ÇALIŞMA AĞACI · DİZİN İLE KARŞILAŞTIRMA")
                    : selectedCommit?.parents.length
                      ? "COMMIT · İLK EBEVEYN İLE KARŞILAŞTIRMA"
                      : "İLK COMMIT · BOŞ AĞAÇ İLE KARŞILAŞTIRMA"}
                </div>
                <div className="diff-file-path" title={activeFile?.path ?? "Dosya seçilmedi"}>
                  <strong>{activeFile ? basename(activeFile.path) : "Dosya seçilmedi"}</strong>
                  {activeFile && parentPath(activeFile.path) && <span>{parentPath(activeFile.path)}</span>}
                </div>
              </div>
            </div>
            {viewScope === "working" && activeFile && (
              <div className="diff-layer-toggle" role="group" aria-label="Fark katmanı">
                <button
                  className={diffLayer === "worktree" ? "active" : ""}
                  type="button"
                  disabled={targetsLocked || !hasStatus(activeFile.worktreeStatus)}
                  onClick={() => {
                    if (mutationInFlight.current || repositoryLoadInFlight.current || diffLayer === "worktree") return;
                    diffRequest.current += 1;
                    setDiffLayer("worktree");
                  }}
                  title="Çalışma ağacı ile dizin arasındaki fark"
                  aria-label="Çalışma ağacı farkını göster"
                  aria-pressed={diffLayer === "worktree"}
                >W</button>
                <button
                  className={diffLayer === "index" ? "active" : ""}
                  type="button"
                  disabled={targetsLocked || !hasStatus(activeFile.indexStatus)}
                  onClick={() => {
                    if (mutationInFlight.current || repositoryLoadInFlight.current || diffLayer === "index") return;
                    diffRequest.current += 1;
                    setDiffLayer("index");
                  }}
                  title="Dizin ile HEAD arasındaki fark"
                  aria-label="Stage edilen farkı göster"
                  aria-pressed={diffLayer === "index"}
                >S</button>
              </div>
            )}
          </div>

          {activeFile && viewScope === "working" && (
            <div className="diff-status-strip">
              <span className={`status-badge ${hasStatus(activeFile.indexStatus) ? "status-active staged" : "status-inactive"}`}>S {fileStatus(activeFile, "index")}</span>
              <span className={`status-badge ${hasStatus(activeFile.worktreeStatus) ? "status-active changed" : "status-inactive"}`}>W {fileStatus(activeFile, "worktree")}</span>
              <span className="diff-status-description">{describeFileStatus(activeFile)}</span>
            </div>
          )}

          {activeFile && viewScope === "history" && selectedCommit && (
            <div className="diff-status-strip history-diff-strip">
              <span className="commit-identity"><GitCommitHorizontal size={13} /> {shortOid(selectedCommit.oid)}</span>
              <span>{selectedCommit.author || "Yazar bilinmiyor"}</span>
              <span className="history-date"><Clock3 size={12} />{formatDate(selectedCommit.date)}</span>
            </div>
          )}

          {(!activeFile || (viewScope === "history" && !selectedCommit)) && <div className="diff-status-strip" />}

          {viewScope === "working" && (
            <div className="selected-file-actions" role="group" aria-label="Seçili dosyanın stage işlemleri">
              <span className="selected-action-label">Seçili dosya</span>
              <button
                className="change-action"
                type="button"
                disabled={!canMutate || selectedStagePaths.length === 0}
                onClick={() => void runMutation("stage", true)}
                aria-label="Seçili dosyayı stage et"
                title={activeFile && isConflicted(activeFile) ? "Bu dosyada çözülmemiş çakışma var" : "Seçili dosyanın çalışma ağacı değişikliğini stage et"}
              ><Plus size={13} />Stage et</button>
              <button
                className="change-action"
                type="button"
                disabled={!canMutate || selectedUnstagePaths.length === 0}
                onClick={() => void runMutation("unstage", true)}
                aria-label="Seçili dosyayı unstage et"
                title={activeFile && isConflicted(activeFile) ? "Bu dosyada çözülmemiş çakışma var" : "Seçili dosyayı unstage et; çalışma dosyasını koru"}
              ><Minus size={13} />Unstage et</button>
            </div>
          )}

          <div className="diff-content">
            {!snapshot && (loadState === "idle" || loadState === "error") ? (
              <div className="panel-message muted-message"><FolderGit2 size={20} /><span>{loadState === "error"
                ? "Depo yolunu kontrol edin veya başka bir depo açın."
                : "Üstteki alana yerel Git deposunun yolunu yazıp Aç’a basın."}</span></div>
            ) : snapshot?.isUnborn && viewScope === "history" && !selectedCommit ? (
              <div className="panel-message muted-message"><GitCommitHorizontal size={19} /> İlk commit henüz oluşturulmadı.</div>
            ) : !activeFile && viewScope === "working" && snapshot?.changes.length === 0 && snapshot ? (
              <div className="panel-message clean-message"><Check size={18} /> Değişiklik yok. Çalışma ağacı temiz.</div>
            ) : viewScope === "history" && selectedCommit && !selectedCommitFiles ? (
              commitFilesError ? <div className="panel-message error-message"><AlertCircle size={18} /><span>{commitFilesError}</span></div> : <div className="panel-message"><LoaderCircle className="spin" size={17} /> Commit dosyaları okunuyor…</div>
            ) : viewScope === "history" && selectedCommit && selectedCommitFiles?.length === 0 ? (
              <div className="panel-message clean-message"><Check size={18} /> Bu commit'te dosya değişikliği yok.</div>
            ) : !activeFile && fileFilterActive && totalFiles > 0 ? (
              <div className="panel-message muted-message"><Search size={18} /> Filtreye uyan dosya yok. Aramayı veya durum filtresini değiştirin.</div>
            ) : !activeFile && viewScope === "history" ? (
              <div className="panel-message muted-message"><History size={18} /> Geçmişten bir commit seçin.</div>
            ) : <DiffView result={diffResult} state={diffState} error={diffError}
              targetKey={JSON.stringify([snapshot ? repositoryIdentity(snapshot.repoPath) : null, viewScope, viewScope === "history" ? selectedCommitOid : diffLayer, selectedPath])}
              partialSelection={partialPreview ? {
                staged: partialPreview.staged,
                selectionToken: partialPreview.result.selectionToken,
                selectableLineIds: partialPreview.result.selectableLineIds,
                unsupportedReason: partialPreview.result.unsupportedReason,
                busy: !canMutate,
                onApply: runPartialSelection,
              } : undefined} />}
          </div>

          <div className="diff-footer">
            <div><span className="diff-legend add-legend">+</span> Ekleme <span className="diff-legend remove-legend">−</span> Silme</div>
            {diffResult && !diffResult.binary && diffResult.text.trim() && (
              <span className="diff-line-total">{diffLineCount} satır</span>
            )}
          </div>
        </section>
      </div>

      <footer className="statusbar">
        <div className="status-left">
          <span className={`status-dot ${snapshot ? "green" : loadState === "error" ? "red" : "idle"}`} />
          <strong>{snapshot?.repoName || "Enogram"}</strong>
          <span className="status-divider" />
          <GitBranch size={13} /><span>{snapshot?.branch || "Dal bekleniyor"}</span>
          <span className="status-divider" />
          <span className="status-detail">{snapshot ? `${unstagedCount} çalışma ağacı · ${stagedCount} dizin` : loadState === "loading" ? "Depo okunuyor…" : "Depo açık değil"}</span>
          {snapshot?.head && <span className="status-oid">HEAD {shortOid(snapshot.head)}</span>}
        </div>
        <div className="status-right">
          {snapshot?.isUnborn && <span className="status-hint"><GitCommitHorizontal size={13} /> Bu dalda commit yok</span>}
          {layoutSaved ? <span className="shortcut-hint">Ctrl+F dosya · Ctrl+Shift+F commit · Ctrl+R yenile</span>
            : <span className="layout-save-warning" role="status" title="Düzen bu oturumda geçerli; ayarlar kaydedilemiyor.">Düzen kaydedilemedi; bu oturumda geçerli.</span>}
        </div>
      </footer>
    </div>
  );
}

function FileRow({
  file,
  scope,
  layer,
  selected,
  disabled,
  onSelect,
}: {
  file: ChangedFile;
  scope: ViewScope;
  layer: DiffLayer;
  selected: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  const staged = scope === "working" && hasStatus(file.indexStatus);
  const changed = scope === "working" && hasStatus(file.worktreeStatus);
  const status = scope === "working" ? fileStatus(file, layer) : (file.indexStatus || file.worktreeStatus || "M").trim();
  const isAdded = status === "A" || status === "?" || status === "??";
  return (
    <button className={`file-row ${selected ? "selected" : ""}`} type="button" onClick={onSelect} disabled={disabled} aria-pressed={selected} title={file.originalPath ? `${file.originalPath} → ${file.path}` : file.path}>
      <span className="file-row-state">
        {scope === "history" ? (
          <span className={`change-glyph ${isAdded ? "added" : "modified"}`}>{isAdded ? "A" : status.slice(0, 1) || "M"}</span>
        ) : (
          <>
            <span className={`layer-glyph staged-glyph ${staged ? "on" : "off"}`} title={`Dizin: ${staged ? file.indexStatus.trim() : "temiz"}`}>{staged ? file.indexStatus.trim().slice(0, 1) : "·"}</span>
            <span className={`layer-glyph worktree-glyph ${changed ? "on" : "off"}`} title={`Çalışma ağacı: ${changed ? file.worktreeStatus.trim() : "temiz"}`}>{changed ? file.worktreeStatus.trim().slice(0, 1) : "·"}</span>
          </>
        )}
      </span>
      <span className="file-row-name">
        {isAdded && <FilePlus2 size={13} className="added-file-icon" />}
        <strong>{basename(file.path)}</strong>
        {file.originalPath && <small>← {basename(file.originalPath)}</small>}
      </span>
      <span className="file-row-path">{parentPath(file.path) || "./"}</span>
      {scope === "working" && selected && (
        <span className="file-selected-layer" aria-label={layer === "index" ? "Dizin farkı" : "Çalışma ağacı farkı"}>
          {layer === "index" ? "S" : "W"}
        </span>
      )}
    </button>
  );
}

function InlineError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="inline-error">
      <AlertCircle size={15} />
      <span>{message}</span>
      <button type="button" onClick={onRetry}>Yeniden dene</button>
    </div>
  );
}

function LoadingRows() {
  return (
    <div className="loading-rows" aria-label="Geçmiş yükleniyor">
      {[0, 1, 2, 3, 4].map((row) => (
        <div className="loading-row" key={row}><span /><i /><i /></div>
      ))}
    </div>
  );
}

export default App;
