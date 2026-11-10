import type { BranchEntry, RemoteEntry, RepositorySnapshot } from "./types";

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

export function networkActionBlockReason(kind: NetworkActionKind, snapshot: RepositorySnapshot | null): string | null {
  if (!snapshot) return "Önce bir yerel depo açın.";
  if (!snapshot.remotes.length) return "Uzak depo tanımlı değil. Uzak depolar penceresinden bir bağlantı ekleyin.";
  if (kind === "fetch") return null;
  if (snapshot.isUnborn || !snapshot.head) return `${kind === "pull" ? "Pull" : "Push"} için önce ilk commit'i oluşturun.`;
  if (kind === "pull" && !currentBranch(snapshot)) return "Pull için yerel bir dala geçin.";
  if (kind === "pull" && (snapshot.changes.length > 0 || snapshot.mergeInProgress)) {
    return "Pull için çalışma ağacı ve dizin temiz olmalı; bekleyen merge tamamlanmalı.";
  }
  return null;
}

export function branchNameError(name: string, existingBranches: BranchEntry[] = []): string | null {
  if (!name.trim()) return "Bir dal adı girin.";
  if (name !== name.trim()) return "Dal adının başında veya sonunda boşluk olamaz.";
  if (existingBranches.some((branch) => branch.name === name)) return "Bu adda bir yerel dal zaten var.";
  return null;
}

export function remoteConfigurationNameError(name: string, existingRemotes: RemoteEntry[] = []): string | null {
  if (!name.trim()) return "Bir uzak depo adı girin.";
  if (name !== name.trim()) return "Uzak depo adının başında veya sonunda boşluk olamaz.";
  if (/[\s\u0000-\u001f\u007f]/u.test(name)) return "Uzak depo adında boşluk veya kontrol karakteri olamaz.";
  if (name.startsWith("-") || name.startsWith("/") || /[:\\*?\[\^~]/u.test(name)) return "Geçerli bir uzak depo adı girin; örneğin origin veya upstream.";
  if (existingRemotes.some((remote) => remote.name === name)) return "Bu adda bir uzak depo zaten var.";
  return null;
}

export function remoteConfigurationUrlError(url: string): string | null {
  if (!url.trim()) return "Bir depo adresi girin.";
  if (url !== url.trim()) return "Adresin başında veya sonunda boşluk olamaz.";
  if (/[\u0000-\u001f\u007f]/u.test(url)) return "Adres satır sonu veya kontrol karakteri içeremez.";
  const scheme = url.match(/^([a-z][a-z0-9+.-]*):\/\//iu)?.[1].toLowerCase();
  if (url.startsWith("-") || (scheme ? !["http", "https", "ssh", "git", "file", "ftp", "ftps"].includes(scheme) : url.includes("::"))) {
    return "SSH, HTTPS veya yerel bir depo adresi girin.";
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

export function remoteConfigurationStaleReason(draft: RemoteConfigurationDraft, snapshot: RepositorySnapshot | null): string | null {
  if (!snapshot || draft.repoPath !== snapshot.repoPath) return "Açık depo değişti. Bu depo için yeni bir form açın.";
  if (draft.source) {
    const current = snapshot.remotes.find((remote) => remote.name === draft.source?.name);
    if (!current) return "Bu uzak depo kaldırılmış. Taslağınız korundu; güncel ayarı yükleyin.";
    if (current.configToken !== draft.source.configToken) return "Uzak depo ayarı bu form açıldıktan sonra değişti. Taslağınız korundu; güncel ayarı yükleyin.";
  }
  if (snapshot.mutationToken !== draft.expectedToken) return "Depo durumu bu form açıldıktan sonra değişti. Taslağınız korundu; güncel ayarı yükleyin.";
  return null;
}

export function remoteConfigurationDraftError(draft: RemoteConfigurationDraft, remotes: RemoteEntry[] = []): string | null {
  if (draft.source?.editBlockedReason) return draft.source.editBlockedReason;
  if (!draft.source) {
    const nameError = remoteConfigurationNameError(draft.name, remotes);
    if (nameError) return nameError;
  }
  const preservesHiddenFetch = draft.source?.configuredFetchUrl === null && draft.fetchUrl === "";
  if (!preservesHiddenFetch) {
    const fetchError = remoteConfigurationUrlError(draft.fetchUrl);
    if (fetchError) return fetchError;
  }
  if (draft.separatePushUrl) {
    const preservesHiddenPush = draft.source?.hasExplicitPushUrl && draft.source.configuredPushUrl === null && draft.pushUrl === "";
    if (!preservesHiddenPush) return remoteConfigurationUrlError(draft.pushUrl);
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

export function repositoryActionContext(action: RepositoryAction): string {
  if (action.kind === "createBranch") return `Yeni dal · ${action.name}${action.checkout ? " · oluştur ve geç" : ""}`;
  if (action.kind === "switchBranch") return `Dal değiştir · ${action.name}`;
  if (action.kind === "addRemote") return `Uzak depo ekle · ${action.name}`;
  if (action.kind === "updateRemote") return `Uzak depo düzenle · ${action.remoteName}`;
  if (action.kind === "fetch") return `Fetch · ${action.remoteName} · izleme dalları`;
  const remoteBranch = `${action.remoteName}/${action.branchName}`;
  return action.kind === "pull"
    ? `Pull · ${remoteBranch} → ${action.localBranch}`
    : `Push · ${action.localBranch} → ${remoteBranch}`;
}

export function repositoryActionProgress(action: RepositoryAction): string {
  switch (action.kind) {
    case "createBranch": return "Dal oluşturuluyor…";
    case "switchBranch": return "Yerel dala geçiliyor…";
    case "addRemote": return "Uzak depo bağlantısı ekleniyor…";
    case "updateRemote": return "Uzak depo adresleri kaydediliyor…";
    case "fetch": return "Uzak dalların izleme bilgileri alınıyor…";
    case "pull": return "Uzak dal alınıyor ve fast-forward uygulanıyor…";
    case "push": return "Yerel dal uzak depoya gönderiliyor…";
  }
}

export function repositoryActionDeadline(kind: RepositoryActionKind): string {
  if (kind === "pull") return "Komut sınırları: ağ 120 sn · fast-forward 20 sn";
  if (kind === "fetch" || kind === "push") return "Ağ komutu sınırı: 120 sn";
  return "Yerel komut sınırı: 20 sn";
}

export function repositoryActionSuccess(action: RepositoryAction): string {
  switch (action.kind) {
    case "createBranch": return `“${action.name}” dalı oluşturuldu${action.checkout ? " ve bu dala geçildi" : ""}.`;
    case "switchBranch": return `“${action.name}” dalına geçildi.`;
    case "addRemote": return `“${action.name}” uzak deposu eklendi. Bağlantıyı kullanmak için Fetch, Pull veya Push seçebilirsiniz.`;
    case "updateRemote": return `“${action.remoteName}” uzak deposunun adresleri kaydedildi.`;
    case "fetch": return `“${action.remoteName}” izleme bilgileri güncellendi.`;
    case "pull": return "Pull tamamlandı. Açık dalın güncel durumu okundu.";
    case "push": return `${action.localBranch} → ${action.remoteName}/${action.branchName} için Push tamamlandı.`;
  }
}
