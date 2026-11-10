import {
  AlertCircle,
  ArrowDownToLine,
  ArrowRight,
  ArrowUpFromLine,
  Check,
  ChevronDown,
  CloudDownload,
  FolderGit2,
  GitBranch,
  GitFork,
  LoaderCircle,
  Plus,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import { type FormEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { RemoteManagerDialog } from "./RemoteManagerDialog";
import { ThemePanel } from "./ThemePanel";
import {
  branchNameError,
  currentBranch,
  defaultRemoteTarget,
  networkActionBlockReason,
  repositoryActionContext,
  repositoryActionDeadline,
  repositoryActionProgress,
  repositoryActionSuccess,
  type NetworkActionKind,
  type RepositoryAction,
  type RepositoryActionFeedback,
} from "./repositoryActionModel";
import { translate, type Locale } from "./i18n.ts";
import { controlsMessages } from "./locales/controls.ts";
import { useI18n } from "./useI18n";
import { repositoryIdentity } from "./repositoryIdentity";
import type { BranchEntry, RepositorySnapshot } from "./types";
import "./repositoryActions.css";

type DialogMode = "branches" | "createBranch" | NetworkActionKind;
type DialogFeedback = Pick<RepositoryActionFeedback, "kind" | "message" | "localizedMessage"> & { messageKey?: keyof typeof controlsMessages };

interface RepositoryActionsProps {
  snapshot: RepositorySnapshot | null;
  targetsLocked: boolean;
  canMutate: boolean;
  refreshing: boolean;
  activeAction: RepositoryAction | null;
  feedback: RepositoryActionFeedback | null;
  remoteManagerRequest?: number;
  density?: "compact" | "comfortable";
  onDensityChange?: (density: "compact" | "comfortable") => void;
  onResetLayout?: () => void;
  onExecute: (action: RepositoryAction) => Promise<RepositoryActionFeedback | null>;
  onDismissFeedback: () => void;
  onRefresh: () => void;
}

const networkActions: NetworkActionKind[] = ["fetch", "pull", "push"];
const networkLabels = { fetch: "Fetch", pull: "Pull", push: "Push" };

function NetworkIcon({ kind, size = 15 }: { kind: NetworkActionKind; size?: number }) {
  if (kind === "fetch") return <CloudDownload size={size} aria-hidden="true" />;
  return kind === "pull"
    ? <ArrowDownToLine size={size} aria-hidden="true" />
    : <ArrowUpFromLine size={size} aria-hidden="true" />;
}

function TrackingCounts({ branch }: { branch: BranchEntry }) {
  const { t } = useI18n(controlsMessages);
  if (branch.ahead === null || branch.behind === null) return null;
  return (
    <span className="branch-tracking-counts" title={t("trackingHint")} aria-label={t("trackingCounts", { ahead: branch.ahead, behind: branch.behind })}>
      <span>↑ {branch.ahead}</span><span>↓ {branch.behind}</span>
    </span>
  );
}

function shortTrackingRef(reference: string): string {
  return reference.replace(/^refs\/(remotes|heads)\//, "");
}

export function RepositoryActions({
  snapshot,
  targetsLocked,
  canMutate,
  refreshing,
  activeAction,
  feedback,
  remoteManagerRequest = 0,
  density,
  onDensityChange,
  onResetLayout,
  onExecute,
  onDismissFeedback,
  onRefresh,
}: RepositoryActionsProps) {
  const { locale, t } = useI18n(controlsMessages);
  const [dialogMode, setDialogMode] = useState<DialogMode | null>(null);
  const [remoteManagerOpen, setRemoteManagerOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [selectedBranch, setSelectedBranch] = useState("");
  const [newBranchName, setNewBranchName] = useState("");
  const [checkoutNewBranch, setCheckoutNewBranch] = useState(true);
  const [remoteName, setRemoteName] = useState("");
  const [remoteBranch, setRemoteBranch] = useState("");
  const [dialogFeedback, setDialogFeedback] = useState<DialogFeedback | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const branchNameRef = useRef<HTMLInputElement>(null);
  const remoteRef = useRef<HTMLSelectElement>(null);
  const dialogRepository = useRef<string | null>(null);
  const lastRemoteRepositoryKey = useRef<string | null>(null);
  const dialogGeneration = useRef(0);
  const submittingRef = useRef(false);
  const handledRemoteManagerRequest = useRef(remoteManagerRequest);
  const dialogId = useId();
  const remoteManagerDialogId = useId();
  const headingId = useId();
  const descriptionId = useId();
  const newBranchId = useId();
  const newBranchHintId = useId();
  const remoteId = useId();
  const remoteBranchId = useId();
  const remoteBranchHintId = useId();
  const branch = snapshot ? currentBranch(snapshot) : null;
  const selectedEntry = snapshot?.branches.find((entry) => entry.name === selectedBranch) ?? null;
  const selectedRemote = snapshot?.remotes.find((remote) => remote.name === remoteName) ?? null;
  const networkMode = dialogMode && networkActions.includes(dialogMode as NetworkActionKind) ? dialogMode as NetworkActionKind : null;
  const isBranchDialog = dialogMode === "branches" || dialogMode === "createBranch";
  const busy = targetsLocked || submitting;
  const canCreate = Boolean(snapshot?.head) && !snapshot?.isUnborn;
  const newNameError = newBranchName ? branchNameError(newBranchName, snapshot?.branches, locale) : null;
  const remoteNameError = remoteBranch ? branchNameError(remoteBranch, [], locale) : null;
  const networkBlock = networkMode ? networkActionBlockReason(networkMode, snapshot, locale) : null;
  const canSwitch = Boolean(selectedEntry && !selectedEntry.current && !selectedEntry.worktreePath);
  const sortedBranches = useMemo(() => [...(snapshot?.branches ?? [])]
    .sort((left, right) => Number(right.current) - Number(left.current) || left.name.localeCompare(right.name))
    .filter((entry) => entry.name.toLowerCase().includes(search.toLowerCase())), [snapshot?.branches, search]);
  const visibleNoticeAction = activeAction ?? feedback?.repositoryAction ?? null;
  const displayedBranch = snapshot?.head && !snapshot.isUnborn && !branch ? t("detachedHead") : snapshot?.branch;
  const pushSource = branch?.name ?? (snapshot?.head ? `HEAD · ${snapshot.head.slice(0, 8)}` : "HEAD");
  const defaultTarget = snapshot ? defaultRemoteTarget(snapshot) : null;
  const remoteRepositoryKey = snapshot ? repositoryIdentity(snapshot.repoPath) : null;

  useEffect(() => {
    // A failed recovery temporarily clears the snapshot, not the user's drafts.
    if (remoteRepositoryKey) lastRemoteRepositoryKey.current = remoteRepositoryKey;
  }, [remoteRepositoryKey]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialogMode || !dialog || dialog.open) return;
    dialog.showModal();
    if (dialogMode === "branches") searchRef.current?.focus();
    else if (dialogMode === "createBranch") branchNameRef.current?.focus();
    else remoteRef.current?.focus();
  }, [dialogMode]);

  useEffect(() => {
    if (!dialogRepository.current || dialogRepository.current === snapshot?.repoPath) return;
    dialogGeneration.current += 1;
    dialogRepository.current = null;
    dialogRef.current?.close();
    setDialogMode(null);
    setRemoteManagerOpen(false);
  }, [snapshot?.repoPath]);

  useEffect(() => {
    if (handledRemoteManagerRequest.current === remoteManagerRequest) return;
    handledRemoteManagerRequest.current = remoteManagerRequest;
    openRemoteManager();
  }, [remoteManagerRequest]);

  function openRemoteManager() {
    if (!snapshot || targetsLocked || !canMutate || submittingRef.current || dialogMode) return;
    dialogRepository.current = snapshot.repoPath;
    setRemoteManagerOpen(true);
  }

  function closeRemoteManager() {
    dialogRepository.current = null;
    setRemoteManagerOpen(false);
  }

  function openDialog(mode: DialogMode) {
    if (!snapshot || targetsLocked || !canMutate || submittingRef.current) return;
    dialogGeneration.current += 1;
    dialogRepository.current = snapshot.repoPath;
    setDialogFeedback(null);
    setSearch("");
    setSelectedBranch(currentBranch(snapshot)?.name ?? snapshot.branches[0]?.name ?? "");
    setNewBranchName("");
    setCheckoutNewBranch(true);
    const target = defaultRemoteTarget(snapshot);
    setRemoteName(target.remoteName);
    setRemoteBranch(target.branchName);
    setDialogMode(mode);
  }

  function closeDialog() {
    dialogGeneration.current += 1;
    dialogRepository.current = null;
    dialogRef.current?.close();
    setDialogMode(null);
  }

  function changeMode(mode: DialogMode) {
    if (busy) return;
    setDialogFeedback(null);
    setDialogMode(mode);
  }

  async function submitAction(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!snapshot || !dialogMode || !canMutate || busy || submittingRef.current) return;
    if (dialogRepository.current !== snapshot.repoPath) return;
    const target = { repoPath: snapshot.repoPath, expectedToken: snapshot.mutationToken };
    let action: RepositoryAction;
    if (dialogMode === "branches") {
      if (!selectedEntry || !canSwitch) return;
      action = { ...target, kind: "switchBranch", name: selectedEntry.name, expectedHead: selectedEntry.head };
    } else if (dialogMode === "createBranch") {
      const validationMessage = (language: Locale) => branchNameError(newBranchName, snapshot.branches, language) ?? translate(controlsMessages, language, "branchFirstCommitRequired");
      const error = branchNameError(newBranchName, snapshot.branches, locale);
      if (error || !canCreate) {
        setDialogFeedback({ kind: "error", message: validationMessage(locale), localizedMessage: { tr: validationMessage("tr"), en: validationMessage("en") } });
        return;
      }
      action = { ...target, kind: "createBranch", name: newBranchName, checkout: checkoutNewBranch };
    } else {
      const validationMessage = (language: Locale) => networkActionBlockReason(dialogMode, snapshot, language)
        ?? (!selectedRemote ? translate(controlsMessages, language, "remoteSelectionRequired") : null)
        ?? (dialogMode !== "fetch" ? branchNameError(remoteBranch, [], language) : null);
      const error = validationMessage(locale);
      if (error || !selectedRemote) {
        setDialogFeedback({ kind: "error", message: error ?? t("remoteNotFound"), localizedMessage: { tr: validationMessage("tr") ?? translate(controlsMessages, "tr", "remoteNotFound"), en: validationMessage("en") ?? translate(controlsMessages, "en", "remoteNotFound") } });
        return;
      }
      const remote = { ...target, remoteName: selectedRemote.name, expectedRemoteToken: selectedRemote.configToken };
      action = dialogMode === "fetch"
        ? { ...remote, kind: "fetch" }
        : { ...remote, kind: dialogMode, branchName: remoteBranch, localBranch: branch?.name ?? pushSource };
    }

    submittingRef.current = true;
    setSubmitting(true);
    setDialogFeedback(null);
    const generation = dialogGeneration.current;
    try {
      const result = await onExecute(action);
      if (generation !== dialogGeneration.current) return;
      if (result?.kind === "success") closeDialog();
      else setDialogFeedback(result ?? { kind: "error", message: t("actionRepositoryChanged"), messageKey: "actionRepositoryChanged" });
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  const submitDisabled = !canMutate || busy || (dialogMode === "branches" ? !canSwitch
    : dialogMode === "createBranch" ? !canCreate || !newBranchName.trim() || Boolean(newNameError)
      : !selectedRemote || Boolean(networkBlock) || (networkMode !== "fetch" && (!remoteBranch.trim() || Boolean(remoteNameError))));
  const submitLabel = dialogMode === "branches" ? t("switchToBranch")
    : dialogMode === "createBranch" ? checkoutNewBranch ? t("createAndSwitch") : t("createBranch")
      : networkMode ? t("startNetworkAction", { action: networkLabels[networkMode] }) : t("startAction");
  const description = isBranchDialog
    ? t("branchesDescription")
    : networkMode === "fetch" ? t("fetchDescription")
      : networkMode === "pull" ? t("pullDescription")
        : t("pushDescription");

  return (
    <>
      <button
        type="button"
        className="branch-context repository-branch-trigger"
        onClick={() => openDialog("branches")}
        disabled={!canMutate || targetsLocked}
        aria-label={`${t("branches")}${displayedBranch ? ` · ${displayedBranch}` : ""}`}
        aria-haspopup="dialog"
        aria-controls={dialogId}
        aria-expanded={isBranchDialog}
        title={snapshot ? t("localBranchesTitle", { branch: displayedBranch ?? "" }) : t("openRepositoryFirst")}
      >
        <GitBranch size={15} aria-hidden="true" />
        <span className="branch-trigger-name">{displayedBranch || t("branchPending")}</span>
        {snapshot?.head && <span className="head-short">{snapshot.head.slice(0, 8)}</span>}
        <ChevronDown className="branch-trigger-chevron" size={12} aria-hidden="true" />
      </button>
      <div className="topbar-actions">
        <ThemePanel density={density} onDensityChange={onDensityChange} onResetLayout={onResetLayout} />
        <button
          className="toolbar-button remote-manager-trigger"
          type="button"
          disabled={!snapshot || !canMutate || targetsLocked}
          onClick={openRemoteManager}
          aria-label={t("remotes")}
          aria-haspopup="dialog"
          aria-controls={remoteManagerDialogId}
          aria-expanded={remoteManagerOpen}
          title={snapshot ? t("remotesTitle") : t("openRepositoryFirst")}
        >
          {activeAction?.kind === "addRemote" || activeAction?.kind === "updateRemote" ? <LoaderCircle className="spin" size={15} aria-hidden="true" /> : <FolderGit2 size={15} aria-hidden="true" />}
          <span>{t("remotes")}</span>
        </button>
        <div className="network-action-group" role="group" aria-label={t("remoteActions")}>
          {networkActions.map((kind) => {
            const blockReason = networkActionBlockReason(kind, snapshot, locale);
            return (
              <button
                className="toolbar-button network-action-button"
                type="button"
                key={kind}
                disabled={!canMutate || targetsLocked || Boolean(blockReason)}
                onClick={() => openDialog(kind)}
                aria-label={networkLabels[kind]}
                aria-haspopup="dialog"
                aria-controls={dialogId}
                aria-expanded={dialogMode === kind}
                title={blockReason ?? t("networkTargetTitle", { action: networkLabels[kind], target: `${defaultTarget?.remoteName}${kind !== "fetch" ? `/${defaultTarget?.branchName}` : ""}` })}
              >
                {activeAction?.kind === kind ? <LoaderCircle className="spin" size={15} aria-hidden="true" /> : <NetworkIcon kind={kind} />}
                <span>{networkLabels[kind]}</span>
              </button>
            );
          })}
        </div>
        <button className="icon-button refresh-button" type="button" onClick={onRefresh} disabled={targetsLocked} aria-label={t("refreshRepository")} title={t("refreshRepositoryShortcut")}>
          {refreshing ? <LoaderCircle className="spin" size={16} /> : <RefreshCw size={16} />}
        </button>
      </div>

      <RemoteManagerDialog
        key={remoteRepositoryKey ?? lastRemoteRepositoryKey.current ?? "no-repository"}
        open={remoteManagerOpen}
        dialogId={remoteManagerDialogId}
        snapshot={snapshot}
        canMutate={canMutate}
        targetsLocked={targetsLocked}
        refreshing={refreshing}
        onExecute={onExecute}
        onClose={closeRemoteManager}
      />

      {visibleNoticeAction && createPortal(
        <div className={`repository-notice ${feedback?.kind ?? "progress"}`} role={feedback?.kind === "error" ? "alert" : "status"} aria-live={feedback?.kind === "error" ? "assertive" : "polite"}>
          {activeAction ? <LoaderCircle className="spin" size={17} aria-hidden="true" />
            : feedback?.kind === "error" ? <AlertCircle size={17} aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}
          <div className="repository-notice-copy">
            <strong>{repositoryActionContext(visibleNoticeAction, locale)}</strong>
            {feedback && <p>{feedback.kind === "success" ? repositoryActionSuccess(feedback.repositoryAction, locale) : feedback.localizedMessage?.[locale] ?? feedback.message}</p>}
            {activeAction && <p>{refreshing ? t("refreshingRepository") : repositoryActionProgress(activeAction, locale)}</p>}
            {activeAction && <small>{repositoryActionDeadline(activeAction.kind, locale)}</small>}
          </div>
          {!activeAction && <button className="subtle-icon-button" type="button" onClick={onDismissFeedback} aria-label={t("dismissActionNotice")} title={t("dismissNotice")}><X size={14} /></button>}
        </div>,
        document.body,
      )}

      <dialog
        ref={dialogRef}
        id={dialogId}
        className="repository-dialog"
        aria-labelledby={headingId}
        aria-describedby={descriptionId}
        onClose={() => {
          if (dialogRef.current?.open) return;
          dialogGeneration.current += 1;
          dialogRepository.current = null;
          setDialogMode(null);
        }}
      >
        <form className="repository-action-form" onSubmit={(event) => void submitAction(event)} noValidate>
          <header className="repository-dialog-header">
            <span className="repository-dialog-icon">{isBranchDialog ? <GitBranch size={21} /> : networkMode && <NetworkIcon kind={networkMode} size={21} />}</span>
            <div>
              <p className="repository-dialog-eyebrow">{snapshot?.repoName ?? "ENOGRAM"} · {isBranchDialog ? t("localBranchesEyebrow") : t("remoteEyebrow")}</p>
              <h2 id={headingId}>{isBranchDialog ? t("branchManagement") : networkMode ? networkLabels[networkMode] : t("repositoryAction")}</h2>
              <p id={descriptionId}>{description}</p>
            </div>
            <button className="icon-button repository-dialog-close" type="button" onClick={closeDialog} aria-label={t("closeActionDialog")} title={t("closeDialog")}><X size={17} /></button>
          </header>

          <div className="repository-action-tabs" role="group" aria-label={isBranchDialog ? t("branchAction") : t("remoteAction")}>
            {isBranchDialog ? (
              <>
                <button type="button" aria-pressed={dialogMode === "branches"} disabled={busy} onClick={() => changeMode("branches")}><GitBranch size={14} />{t("localBranches")}<span>{snapshot?.branches.length ?? 0}</span></button>
                <button type="button" aria-pressed={dialogMode === "createBranch"} disabled={busy} onClick={() => changeMode("createBranch")}><Plus size={14} />{t("newBranch")}</button>
              </>
            ) : networkActions.map((kind) => (
              <button key={kind} type="button" aria-pressed={dialogMode === kind} disabled={busy || Boolean(networkActionBlockReason(kind, snapshot, locale))} title={networkActionBlockReason(kind, snapshot, locale) ?? networkLabels[kind]} onClick={() => changeMode(kind)}><NetworkIcon kind={kind} size={14} />{networkLabels[kind]}</button>
            ))}
          </div>

          <div className="repository-dialog-main">
            {dialogMode === "branches" && (
              <>
                <label className="branch-search"><Search size={15} aria-hidden="true" /><input ref={searchRef} value={search} onChange={(event) => setSearch(event.target.value)} disabled={busy} placeholder={t("searchBranchesPlaceholder")} aria-label={t("searchBranches")} spellCheck={false} /></label>
                <div className="local-branch-list" role="group" aria-label={t("localBranches")}>
                  {sortedBranches.map((entry) => {
                    const occupied = Boolean(entry.worktreePath) && !entry.current;
                    return (
                      <button type="button" className={`local-branch-row ${selectedBranch === entry.name ? "selected" : ""}`} key={entry.name} aria-pressed={selectedBranch === entry.name} disabled={busy || occupied} onClick={() => { setSelectedBranch(entry.name); setDialogFeedback(null); }} title={occupied ? t("occupiedBranchTitle", { path: entry.worktreePath ?? "" }) : entry.name}>
                        <GitBranch size={16} aria-hidden="true" />
                        <span className="local-branch-copy"><strong>{entry.name}</strong><small>{occupied ? entry.worktreePath : entry.upstream ? shortTrackingRef(entry.upstream) : t("upstreamMissing")}</small></span>
                        <span className="local-branch-meta"><span className={`local-branch-badge ${entry.current ? "current" : ""}`}>{entry.current ? t("branchCurrent") : occupied ? t("branchOtherWorktree") : entry.head.slice(0, 8)}</span><TrackingCounts branch={entry} /></span>
                        {entry.current && <Check size={13} className="local-branch-current" aria-hidden="true" />}
                      </button>
                    );
                  })}
                  {!sortedBranches.length && <div className="repository-empty"><GitBranch size={22} /><span>{search ? t("noMatchingBranches") : snapshot?.isUnborn ? t("branchesAfterFirstCommit") : t("noLocalBranches")}</span></div>}
                </div>
                <p className="repository-field-hint branch-switch-hint">{snapshot?.changes.length ? t("switchWithChangesHint") : t("switchBranchHint")}</p>
              </>
            )}

            {dialogMode === "createBranch" && (
              <div className="repository-fields">
                <div className="repository-origin"><GitFork size={17} /><div><span>{t("branchStartPoint")}</span><strong>{displayedBranch || "HEAD"}{snapshot?.head && <code>{snapshot.head.slice(0, 8)}</code>}</strong></div></div>
                {!canCreate && <div className="repository-form-message warning" role="status"><AlertCircle size={15} /><span>{t("newBranchFirstCommit")}</span></div>}
                <div className="repository-field"><label htmlFor={newBranchId}>{t("newBranchName")}</label><input ref={branchNameRef} id={newBranchId} value={newBranchName} onChange={(event) => { setNewBranchName(event.target.value); setDialogFeedback(null); }} placeholder={t("newBranchPlaceholder")} disabled={busy || !canCreate} autoComplete="off" spellCheck={false} aria-invalid={Boolean(newNameError)} aria-describedby={newBranchHintId} /><p id={newBranchHintId} className={`repository-field-hint ${newNameError ? "error" : ""}`}>{newNameError ?? t("branchHeadHint")}</p></div>
                <label className="repository-checkbox"><input type="checkbox" checked={checkoutNewBranch} onChange={(event) => setCheckoutNewBranch(event.target.checked)} disabled={busy || !canCreate} /><span><strong>{t("checkoutNewBranch")}</strong><small>{t("checkoutChangesHint")}</small></span></label>
              </div>
            )}

            {networkMode && (
              <div className="repository-fields">
                <div className="repository-target-flow" aria-label={t("actionTarget")}>
                  <div><span>{networkMode === "push" ? t("localSource") : t("remoteSource")}</span><strong>{networkMode === "push" ? pushSource : remoteName ? `${remoteName}${networkMode !== "fetch" && remoteBranch ? `/${remoteBranch}` : ""}` : t("selectRemote")}</strong></div>
                  <ArrowRight size={18} aria-hidden="true" />
                  <div><span>{networkMode === "push" ? t("remoteTarget") : t("localTarget")}</span><strong>{networkMode === "push" ? `${remoteName}/${remoteBranch || "…"}` : networkMode === "fetch" ? t("trackingBranches") : branch?.name ?? t("noLocalBranch")}</strong></div>
                </div>
                <div className="repository-remote-fields">
                  <div className="repository-field"><label htmlFor={remoteId}>{t("remote")}</label><select ref={remoteRef} id={remoteId} value={selectedRemote ? remoteName : ""} disabled={busy || !snapshot?.remotes.length} onChange={(event) => { setRemoteName(event.target.value); setDialogFeedback(null); }}><option value="" disabled>{t("selectRemote")}</option>{snapshot?.remotes.map((remote) => <option key={remote.name} value={remote.name}>{remote.name}</option>)}</select></div>
                  {networkMode !== "fetch" && <div className="repository-field"><label htmlFor={remoteBranchId}>{t("remoteBranch")}</label><input id={remoteBranchId} value={remoteBranch} onChange={(event) => { setRemoteBranch(event.target.value); setDialogFeedback(null); }} placeholder={branch?.name || "main"} disabled={busy} spellCheck={false} autoComplete="off" aria-invalid={Boolean(remoteNameError)} aria-describedby={remoteBranchHintId} /></div>}
                </div>
                {selectedRemote && <div className="repository-remote-url"><FolderGit2 size={13} aria-hidden="true" /><span>{networkMode === "push" ? t("pushAddress") : t("fetchAddress")}</span><code>{networkMode === "push" ? selectedRemote.pushUrl : selectedRemote.fetchUrl}</code></div>}
                {networkMode !== "fetch" && <p id={remoteBranchHintId} className={`repository-field-hint ${remoteNameError ? "error" : ""}`}>{remoteNameError ?? t("remoteBranchHint", { branch: remoteBranch || "…" })}</p>}
                {branch && <div className="repository-tracking-summary"><GitBranch size={13} /><span>{branch.upstream ? t("upstreamLabel", { reference: shortTrackingRef(branch.upstream) }) : t("currentUpstreamMissing")}</span><TrackingCounts branch={branch} /></div>}
                {networkMode === "pull" && <p className="repository-field-hint">{t("pullCleanHint")}</p>}
                {networkBlock && <div className="repository-form-message warning" role="status"><AlertCircle size={15} /><span>{networkBlock}</span></div>}
              </div>
            )}

            {dialogFeedback && <div className={`repository-form-message ${dialogFeedback.kind}`} role={dialogFeedback.kind === "error" ? "alert" : "status"}><AlertCircle size={15} /><span>{dialogFeedback.messageKey ? t(dialogFeedback.messageKey) : dialogFeedback.localizedMessage?.[locale] ?? dialogFeedback.message}</span></div>}
            {submitting && <div className="repository-form-message progress" role="status"><LoaderCircle className="spin" size={15} /><span>{refreshing ? t("refreshingRepository") : activeAction ? repositoryActionProgress(activeAction, locale) : t("startingAction")}<small>{t("closingDoesNotCancel")}</small></span></div>}
          </div>

          <footer className="repository-dialog-footer">
            <span>{repositoryActionDeadline(networkMode ?? "createBranch", locale)}</span>
            <button type="button" className="change-action" onClick={closeDialog}>{submitting ? t("closeDialog") : t("close")}</button>
            <button type="submit" className="commit-button repository-execute" disabled={submitDisabled}>{submitting ? <LoaderCircle className="spin" size={14} /> : networkMode ? <NetworkIcon kind={networkMode} size={14} /> : dialogMode === "createBranch" ? <Plus size={14} /> : <GitBranch size={14} />}{submitting ? t("actionRunning") : submitLabel}</button>
          </footer>
        </form>
      </dialog>
    </>
  );
}
