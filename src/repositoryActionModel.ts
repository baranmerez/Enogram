import type { BranchEntry, RemoteEntry, RepositorySnapshot } from "./types";
import { getLocale, translate, type Locale } from "./i18n.ts";
import { controlsMessages } from "./locales/controls.ts";
import { localizeBackendText } from "./localizeBackendText.ts";

export type NetworkActionKind = "fetch" | "pull" | "push";
export type RemoteConfigurationActionKind = "addRemote" | "updateRemote";
export type RepositoryActionKind = "createBranch" | "switchBranch" | NetworkActionKind | RemoteConfigurationActionKind;

interface RepositoryTarget {
  repoPath: string;
  expectedToken: string;
}

interface RemoteTarget extends RepositoryTarget {
  remoteName: string;
  expectedRemoteToken: string;
}

export type RemoteConfigurationAction =
  | (RepositoryTarget & { kind: "addRemote"; name: string; fetchUrl: string; pushUrl: string | null })
  | (RemoteTarget & { kind: "updateRemote"; fetchUrl: string | null; pushUrl: string | null; updatePushUrl: boolean });

export type RepositoryAction =
  | (RepositoryTarget & { kind: "createBranch"; name: string; checkout: boolean })
  | (RepositoryTarget & { kind: "switchBranch"; name: string; expectedHead: string })
  | (RemoteTarget & { kind: "fetch" })
  | (RemoteTarget & { kind: "pull" | "push"; branchName: string; localBranch: string })
  | RemoteConfigurationAction;

export interface RemoteConfigurationDraft extends RepositoryTarget {
  source: RemoteEntry | null;
  name: string;
  fetchUrl: string;
  pushUrl: string;
  separatePushUrl: boolean;
}

export interface RepositoryActionFeedback {
  kind: "success" | "error";
  message: string;
  localizedMessage?: Record<Locale, string>;
  repositoryAction: RepositoryAction;
}

export function currentBranch(snapshot: RepositorySnapshot): BranchEntry | null {
  return snapshot.branches.find((branch) => branch.current) ?? null;
}

export function defaultRemoteTarget(snapshot: RepositorySnapshot): { remoteName: string; branchName: string } {
  const branch = currentBranch(snapshot);
  const upstreamRemote = snapshot.remotes.find((remote) => remote.name === branch?.upstreamRemote);
  return {
    remoteName: (upstreamRemote ?? snapshot.remotes[0])?.name ?? "",
    branchName: upstreamRemote && branch?.upstreamBranch
      ? branch.upstreamBranch.replace(/^refs\/heads\//, "")
      : branch?.name ?? (snapshot.isUnborn ? snapshot.branch : ""),
  };
}

export function networkActionBlockReason(kind: NetworkActionKind, snapshot: RepositorySnapshot | null, locale: Locale = getLocale()): string | null {
  if (!snapshot) return translate(controlsMessages, locale, "openRepositoryFirst");
  if (!snapshot.remotes.length) return translate(controlsMessages, locale, "noRemotes");
  if (kind === "fetch") return null;
  if (snapshot.isUnborn || !snapshot.head) return translate(controlsMessages, locale, "networkFirstCommitRequired", { action: kind === "pull" ? "Pull" : "Push" });
  if (kind === "pull" && !currentBranch(snapshot)) return translate(controlsMessages, locale, "pullLocalBranchRequired");
  if (kind === "pull" && (snapshot.changes.length > 0 || snapshot.mergeInProgress)) {
    return translate(controlsMessages, locale, "pullCleanCheckoutRequired");
  }
  return null;
}

export function branchNameError(name: string, existingBranches: BranchEntry[] = [], locale: Locale = getLocale()): string | null {
  if (!name.trim()) return translate(controlsMessages, locale, "branchNameRequired");
  if (name !== name.trim()) return translate(controlsMessages, locale, "branchNameWhitespace");
  if (existingBranches.some((branch) => branch.name === name)) return translate(controlsMessages, locale, "branchNameExists");
  return null;
}

export function remoteConfigurationNameError(name: string, existingRemotes: RemoteEntry[] = [], locale: Locale = getLocale()): string | null {
  if (!name.trim()) return translate(controlsMessages, locale, "remoteNameRequired");
  if (name !== name.trim()) return translate(controlsMessages, locale, "remoteNameWhitespace");
  if (/[\s\u0000-\u001f\u007f]/u.test(name)) return translate(controlsMessages, locale, "remoteNameCharacters");
  if (name.startsWith("-") || name.startsWith("/") || /[:\\*?\[\^~]/u.test(name)) return translate(controlsMessages, locale, "remoteNameInvalid");
  if (existingRemotes.some((remote) => remote.name === name)) return translate(controlsMessages, locale, "remoteNameExists");
  return null;
}

export function remoteConfigurationUrlError(url: string, locale: Locale = getLocale()): string | null {
  if (!url.trim()) return translate(controlsMessages, locale, "remoteUrlRequired");
  if (url !== url.trim()) return translate(controlsMessages, locale, "remoteUrlWhitespace");
  if (/[\u0000-\u001f\u007f]/u.test(url)) return translate(controlsMessages, locale, "remoteUrlCharacters");
  const scheme = url.match(/^([a-z][a-z0-9+.-]*):\/\//iu)?.[1].toLowerCase();
  if (url.startsWith("-") || (scheme ? !["http", "https", "ssh", "git", "file", "ftp", "ftps"].includes(scheme) : url.includes("::"))) {
    return translate(controlsMessages, locale, "remoteUrlInvalid");
  }
  return null;
}

export function createRemoteConfigurationDraft(snapshot: RepositorySnapshot, remote: RemoteEntry | null = null): RemoteConfigurationDraft {
  return {
    repoPath: snapshot.repoPath,
    expectedToken: snapshot.mutationToken,
    source: remote ? { ...remote } : null,
    name: remote?.name ?? (snapshot.remotes.length ? "" : "origin"),
    // Resolved display URLs can contain redaction or Git URL rewrites. They are never editable defaults.
    fetchUrl: remote?.configuredFetchUrl ?? "",
    pushUrl: remote?.configuredPushUrl ?? "",
    separatePushUrl: remote?.hasExplicitPushUrl ?? false,
  };
}

export function remoteConfigurationStaleReason(draft: RemoteConfigurationDraft, snapshot: RepositorySnapshot | null, locale: Locale = getLocale()): string | null {
  if (!snapshot || draft.repoPath !== snapshot.repoPath) return translate(controlsMessages, locale, "remoteDraftRepositoryChanged");
  if (draft.source) {
    const current = snapshot.remotes.find((remote) => remote.name === draft.source?.name);
    if (!current) return translate(controlsMessages, locale, "remoteDraftRemoved");
    if (current.configToken !== draft.source.configToken) return translate(controlsMessages, locale, "remoteDraftConfigurationChanged");
  }
  if (snapshot.mutationToken !== draft.expectedToken) return translate(controlsMessages, locale, "remoteDraftStateChanged");
  return null;
}

export function remoteConfigurationDraftError(draft: RemoteConfigurationDraft, remotes: RemoteEntry[] = [], locale: Locale = getLocale()): string | null {
  if (draft.source?.editBlockedReason) return localizeBackendText(draft.source.editBlockedReason, locale);
  if (!draft.source) {
    const nameError = remoteConfigurationNameError(draft.name, remotes, locale);
    if (nameError) return nameError;
  }
  const preservesHiddenFetch = draft.source?.configuredFetchUrl === null && draft.fetchUrl === "";
  if (!preservesHiddenFetch) {
    const fetchError = remoteConfigurationUrlError(draft.fetchUrl, locale);
    if (fetchError) return fetchError;
  }
  if (draft.separatePushUrl) {
    const preservesHiddenPush = draft.source?.hasExplicitPushUrl && draft.source.configuredPushUrl === null && draft.pushUrl === "";
    if (!preservesHiddenPush) return remoteConfigurationUrlError(draft.pushUrl, locale);
  }
  return null;
}

export function remoteConfigurationAction(draft: RemoteConfigurationDraft): RemoteConfigurationAction {
  const target = { repoPath: draft.repoPath, expectedToken: draft.expectedToken };
  if (!draft.source) {
    return { ...target, kind: "addRemote", name: draft.name, fetchUrl: draft.fetchUrl, pushUrl: draft.separatePushUrl ? draft.pushUrl : null };
  }
  const fetchUrl = draft.fetchUrl === (draft.source.configuredFetchUrl ?? "") ? null : draft.fetchUrl;
  const updatePushUrl = draft.separatePushUrl !== draft.source.hasExplicitPushUrl
    || (draft.separatePushUrl && draft.pushUrl !== (draft.source.configuredPushUrl ?? ""));
  return {
    ...target,
    kind: "updateRemote",
    remoteName: draft.source.name,
    expectedRemoteToken: draft.source.configToken,
    fetchUrl,
    pushUrl: updatePushUrl && draft.separatePushUrl ? draft.pushUrl : null,
    updatePushUrl,
  };
}

export function remoteConfigurationHasChanges(draft: RemoteConfigurationDraft): boolean {
  const action = remoteConfigurationAction(draft);
  return action.kind === "addRemote" || action.fetchUrl !== null || action.updatePushUrl;
}

export function repositoryActionCommand(action: RepositoryAction): { command: string; args: Record<string, unknown> } {
  const target = { repoPath: action.repoPath, expectedToken: action.expectedToken };
  if (action.kind === "createBranch") {
    return { command: "create_branch", args: { ...target, name: action.name, checkout: action.checkout } };
  }
  if (action.kind === "switchBranch") {
    return { command: "switch_branch", args: { ...target, name: action.name, expectedHead: action.expectedHead } };
  }
  if (action.kind === "addRemote") {
    return { command: "add_remote", args: { ...target, name: action.name, fetchUrl: action.fetchUrl, pushUrl: action.pushUrl } };
  }
  const remote = { ...target, remoteName: action.remoteName, expectedRemoteToken: action.expectedRemoteToken };
  if (action.kind === "updateRemote") {
    return { command: "update_remote", args: { ...remote, fetchUrl: action.fetchUrl, pushUrl: action.pushUrl, updatePushUrl: action.updatePushUrl } };
  }
  return action.kind === "fetch"
    ? { command: "fetch_remote", args: remote }
    : { command: action.kind === "pull" ? "pull_remote" : "push_remote", args: { ...remote, branchName: action.branchName } };
}

export function repositoryActionContext(action: RepositoryAction, locale: Locale = getLocale()): string {
  if (action.kind === "createBranch") return translate(controlsMessages, locale, action.checkout ? "actionCreateAndSwitchContext" : "actionCreateBranchContext", { name: action.name });
  if (action.kind === "switchBranch") return translate(controlsMessages, locale, "actionSwitchBranchContext", { name: action.name });
  if (action.kind === "addRemote") return translate(controlsMessages, locale, "actionAddRemoteContext", { name: action.name });
  if (action.kind === "updateRemote") return translate(controlsMessages, locale, "actionUpdateRemoteContext", { name: action.remoteName });
  if (action.kind === "fetch") return translate(controlsMessages, locale, "actionFetchContext", { name: action.remoteName });
  const remoteBranch = `${action.remoteName}/${action.branchName}`;
  return translate(controlsMessages, locale, action.kind === "pull" ? "actionPullContext" : "actionPushContext", { remoteBranch, localBranch: action.localBranch });
}

export function repositoryActionProgress(action: RepositoryAction, locale: Locale = getLocale()): string {
  switch (action.kind) {
    case "createBranch": return translate(controlsMessages, locale, "actionCreateBranchProgress");
    case "switchBranch": return translate(controlsMessages, locale, "actionSwitchBranchProgress");
    case "addRemote": return translate(controlsMessages, locale, "actionAddRemoteProgress");
    case "updateRemote": return translate(controlsMessages, locale, "actionUpdateRemoteProgress");
    case "fetch": return translate(controlsMessages, locale, "actionFetchProgress");
    case "pull": return translate(controlsMessages, locale, "actionPullProgress");
    case "push": return translate(controlsMessages, locale, "actionPushProgress");
  }
}

export function repositoryActionDeadline(kind: RepositoryActionKind, locale: Locale = getLocale()): string {
  if (kind === "pull") return translate(controlsMessages, locale, "actionPullDeadline");
  if (kind === "fetch" || kind === "push") return translate(controlsMessages, locale, "actionNetworkDeadline");
  return translate(controlsMessages, locale, "actionLocalDeadline");
}

export function repositoryActionSuccess(action: RepositoryAction, locale: Locale = getLocale()): string {
  switch (action.kind) {
    case "createBranch": return translate(controlsMessages, locale, action.checkout ? "actionCreateAndSwitchSuccess" : "actionCreateBranchSuccess", { name: action.name });
    case "switchBranch": return translate(controlsMessages, locale, "actionSwitchBranchSuccess", { name: action.name });
    case "addRemote": return translate(controlsMessages, locale, "actionAddRemoteSuccess", { name: action.name });
    case "updateRemote": return translate(controlsMessages, locale, "actionUpdateRemoteSuccess", { name: action.remoteName });
    case "fetch": return translate(controlsMessages, locale, "actionFetchSuccess", { name: action.remoteName });
    case "pull": return translate(controlsMessages, locale, "actionPullSuccess");
    case "push": return translate(controlsMessages, locale, "actionPushSuccess", { localBranch: action.localBranch, remoteBranch: `${action.remoteName}/${action.branchName}` });
  }
}
