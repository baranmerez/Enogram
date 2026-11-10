import { AlertCircle, Check, ChevronRight, FolderGit2, Link2, LoaderCircle, LockKeyhole, Plus, RefreshCw, Save, X } from "lucide-react";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import {
  createRemoteConfigurationDraft,
  defaultRemoteTarget,
  remoteConfigurationAction,
  remoteConfigurationDraftError,
  remoteConfigurationHasChanges,
  remoteConfigurationNameError,
  remoteConfigurationStaleReason,
  remoteConfigurationUrlError,
  type RemoteConfigurationDraft,
  type RepositoryAction,
  type RepositoryActionFeedback,
} from "./repositoryActionModel";
import { repositoryIdentity } from "./repositoryIdentity";
import type { RemoteEntry, RepositorySnapshot } from "./types";

interface RemoteManagerDialogProps {
  open: boolean;
  dialogId: string;
  snapshot: RepositorySnapshot | null;
  canMutate: boolean;
  targetsLocked: boolean;
  refreshing: boolean;
  onExecute: (action: RepositoryAction) => Promise<RepositoryActionFeedback | null>;
  onClose: () => void;
}

interface RemoteManagerState {
  selectedKey: string;
  drafts: Record<string, RemoteConfigurationDraft>;
}

function remoteDraftKey(remote: RemoteEntry | null): string {
  return remote ? `edit:${remote.name}` : "add";
}

function CurrentRemoteAddress({ address }: { address: string }) {
  return <div className="remote-current-address"><span>Git'in kullandığı mevcut adres</span><code>{address}</code></div>;
}

export function RemoteManagerDialog({ open, dialogId, snapshot, canMutate, targetsLocked, refreshing, onExecute, onClose }: RemoteManagerDialogProps) {
  const [state, setState] = useState<RemoteManagerState | null>(null);
  const [feedback, setFeedback] = useState<Pick<RepositoryActionFeedback, "kind" | "message"> | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const fetchRef = useRef<HTMLInputElement>(null);
  const editorHeadingRef = useRef<HTMLHeadingElement>(null);
  const submittingRef = useRef(false);
  const generation = useRef(0);
  const headingId = useId();
  const descriptionId = useId();
  const editorHeadingId = useId();
  const nameId = useId();
  const nameHintId = useId();
  const fetchId = useId();
  const fetchHintId = useId();
  const pushId = useId();
  const pushHintId = useId();
  const draft = state?.drafts[state.selectedKey] ?? null;
  // Reconcile a worktree's path spelling without rebasing its captured tokens.
  const targetDraft = draft && snapshot && draft.repoPath !== snapshot.repoPath
    && repositoryIdentity(draft.repoPath) === repositoryIdentity(snapshot.repoPath)
    ? { ...draft, repoPath: snapshot.repoPath } : draft;
  const source = draft?.source ?? null;
  const busy = submitting || targetsLocked;
  const blockedReason = source?.editBlockedReason ?? null;
  const staleReason = targetDraft ? remoteConfigurationStaleReason(targetDraft, snapshot) : null;
  const fieldDisabled = busy || !canMutate || Boolean(blockedReason);
  const nameError = draft && !source && draft.name ? remoteConfigurationNameError(draft.name, snapshot?.remotes) : null;
  const fetchHidden = source?.configuredFetchUrl === null;
  const pushHidden = Boolean(source?.hasExplicitPushUrl && source.configuredPushUrl === null);
  const fetchError = draft && (draft.fetchUrl || (source && !fetchHidden)) ? remoteConfigurationUrlError(draft.fetchUrl) : null;
  const pushError = draft?.separatePushUrl && (draft.pushUrl || (source?.hasExplicitPushUrl && !pushHidden)) ? remoteConfigurationUrlError(draft.pushUrl) : null;
  const formError = draft ? remoteConfigurationDraftError(draft, snapshot?.remotes) : null;
  const hasChanges = draft ? remoteConfigurationHasChanges(draft) : false;

  useEffect(() => {
    if (!open || !snapshot || draft) return;
    const initialRemote = snapshot.remotes.find((remote) => remote.name === defaultRemoteTarget(snapshot).remoteName) ?? null;
    const selectedKey = remoteDraftKey(initialRemote);
    setState((current) => ({
      selectedKey,
      drafts: { ...current?.drafts, [selectedKey]: createRemoteConfigurationDraft(snapshot, initialRemote) },
    }));
  }, [open, snapshot, draft]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (!open) {
      if (dialog.open) dialog.close();
      return;
    }
    if (!draft) return;
    if (!dialog.open) {
      generation.current += 1;
      dialog.showModal();
    }
    if (draft.source?.editBlockedReason) editorHeadingRef.current?.focus();
    else if (draft.source) fetchRef.current?.focus();
    else nameRef.current?.focus();
  }, [open, state?.selectedKey, draft?.expectedToken, draft?.source?.configToken, Boolean(draft)]);

  function closeDialog() {
    generation.current += 1;
    dialogRef.current?.close();
    onClose();
  }

  function selectDraft(remote: RemoteEntry | null) {
    if (!snapshot || busy || submittingRef.current) return;
    const selectedKey = remoteDraftKey(remote);
    setState((current) => ({
      selectedKey,
      drafts: { ...current?.drafts, [selectedKey]: current?.drafts[selectedKey] ?? createRemoteConfigurationDraft(snapshot, remote) },
    }));
    setFeedback(null);
  }

  function updateDraft(change: Partial<Pick<RemoteConfigurationDraft, "name" | "fetchUrl" | "pushUrl" | "separatePushUrl">>) {
    if (fieldDisabled || submittingRef.current) return;
    setState((current) => {
      if (!current) return current;
      const currentDraft = current.drafts[current.selectedKey];
      if (!currentDraft) return current;
      return { ...current, drafts: { ...current.drafts, [current.selectedKey]: { ...currentDraft, ...change } } };
    });
    setFeedback(null);
  }

  function reloadDraft() {
    if (!snapshot || !draft || busy || submittingRef.current) return;
    const currentRemote = source ? snapshot.remotes.find((remote) => remote.name === source.name) ?? null : null;
    const selectedKey = remoteDraftKey(currentRemote);
    setState((current) => ({
      selectedKey,
      drafts: { ...current?.drafts, [selectedKey]: createRemoteConfigurationDraft(snapshot, currentRemote) },
    }));
    setFeedback(null);
  }

  async function submitAction(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!draft || !targetDraft || !state || !snapshot || !canMutate || busy || submittingRef.current) return;
    const error = remoteConfigurationStaleReason(targetDraft, snapshot) ?? remoteConfigurationDraftError(draft, snapshot.remotes);
    if (error) {
      setFeedback({ kind: "error", message: error });
      return;
    }
    if (!remoteConfigurationHasChanges(draft)) return;
    const action = remoteConfigurationAction(targetDraft);
    const submittedKey = state.selectedKey;
    const submittedDraft = draft;
    const submittedGeneration = generation.current;
    submittingRef.current = true;
    setSubmitting(true);
    setFeedback(null);
    try {
      const result = await onExecute(action);
      if (result?.kind === "success") {
        // Clear only the saved draft. Other remote drafts and a dismissed failed form stay intact.
        setState((current) => {
          if (!current || current.drafts[submittedKey] !== submittedDraft) return current;
          const drafts = { ...current.drafts };
          delete drafts[submittedKey];
          return { ...current, drafts };
        });
        if (submittedGeneration === generation.current) closeDialog();
      } else if (submittedGeneration === generation.current) {
        setFeedback(result ?? { kind: "error", message: "Depo durumu değişti. Taslağınız korundu; güncel ayarı yükleyip yeniden düzenleyin." });
      }
    } catch {
      if (submittedGeneration === generation.current) {
        setFeedback({ kind: "error", message: "Uzak depo ayarı kaydedilemedi. Taslağınız korundu. Depo durumunu yenileyip tekrar kontrol edin." });
      }
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <dialog
      ref={dialogRef}
      id={dialogId}
      className="repository-dialog remote-manager-dialog"
      aria-labelledby={headingId}
      aria-describedby={descriptionId}
      onClose={() => {
        if (dialogRef.current?.open) return;
        generation.current += 1;
        onClose();
      }}
    >
      <form className="repository-action-form" onSubmit={(event) => void submitAction(event)} noValidate>
        <header className="repository-dialog-header">
          <span className="repository-dialog-icon"><FolderGit2 size={21} aria-hidden="true" /></span>
          <div>
            <p className="repository-dialog-eyebrow">{snapshot?.repoName ?? "ENOGRAM"} · BAĞLANTI AYARLARI</p>
            <h2 id={headingId}>Uzak depolar</h2>
            <p id={descriptionId}>Fetch ve Push adreslerini bu yerel depo için yönetin.</p>
          </div>
          <button className="icon-button repository-dialog-close" type="button" onClick={closeDialog} aria-label="Uzak depolar penceresini kapat" title="Pencereyi kapat"><X size={17} /></button>
        </header>

        <div className="remote-manager-layout">
          <nav className="remote-manager-navigation" aria-label="Uzak depo seçimi">
            <div className="remote-navigation-label"><span>TANIMLI DEPOLAR</span><span>{snapshot?.remotes.length ?? 0}</span></div>
            <div className="remote-manager-list" role="group" aria-label="Tanımlı uzak depolar">
              {snapshot?.remotes.map((remote) => (
                <button
                  key={remote.name}
                  type="button"
                  className={`remote-manager-row ${state?.selectedKey === remoteDraftKey(remote) ? "selected" : ""}`}
                  aria-pressed={state?.selectedKey === remoteDraftKey(remote)}
                  disabled={busy}
                  onClick={() => selectDraft(remote)}
                  title={remote.name}
                >
                  <FolderGit2 size={16} aria-hidden="true" />
                  <span><strong>{remote.name}</strong><small>{remote.editBlockedReason ? "Yalnızca görüntüleme" : remote.hasExplicitPushUrl ? "Ayrı Push adresi" : "Fetch ve Push"}</small></span>
                  {remote.editBlockedReason ? <LockKeyhole size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
                </button>
              ))}
              {!snapshot?.remotes.length && <div className="remote-list-empty"><Link2 size={21} aria-hidden="true" /><span>Henüz uzak depo yok.</span><small>İlk bağlantınızı ekleyin.</small></div>}
            </div>
            <button className={`remote-manager-add ${state?.selectedKey === "add" ? "selected" : ""}`} type="button" aria-pressed={state?.selectedKey === "add"} disabled={busy} onClick={() => selectDraft(null)}><Plus size={15} aria-hidden="true" /><span>Yeni uzak depo</span></button>
            <p className="remote-navigation-note">Bağlantılar bu deponun Git ayarlarında saklanır.</p>
          </nav>

          <div className="repository-dialog-main remote-manager-editor">
            {draft && (
              <section className="repository-fields" aria-labelledby={editorHeadingId}>
                <div className="remote-editor-heading">
                  <div><p>{source ? "BAĞLANTIYI DÜZENLE" : "YENİ BAĞLANTI"}</p><h3 ref={editorHeadingRef} tabIndex={-1} id={editorHeadingId}>{source ? source.name : "Uzak depo ekle"}</h3></div>
                  <span><span className="remote-local-dot" />Yerel ayar</span>
                </div>

                {blockedReason && <div className="repository-form-message warning" role="status"><LockKeyhole size={15} aria-hidden="true" /><span>{blockedReason}</span></div>}

                {staleReason && !submitting && <div className="repository-form-message warning remote-stale-message" role="status"><AlertCircle size={15} aria-hidden="true" /><span>{staleReason}<button className="change-action" type="button" disabled={busy} onClick={reloadDraft}><RefreshCw size={13} aria-hidden="true" />Güncel ayarı yükle</button><small>Bu formdaki değişikliklerin yerine güncel değerler yüklenir.</small></span></div>}

                <div className="repository-field">
                  <label htmlFor={nameId}>Uzak depo adı{source && <LockKeyhole size={11} aria-hidden="true" />}</label>
                  <input ref={nameRef} id={nameId} value={source?.name ?? draft.name} onChange={(event) => updateDraft({ name: event.target.value })} readOnly={Boolean(source)} disabled={fieldDisabled} placeholder="origin" autoComplete="off" autoCapitalize="none" spellCheck={false} aria-invalid={Boolean(nameError)} aria-describedby={nameHintId} />
                  <p id={nameHintId} className={`repository-field-hint ${nameError ? "error" : ""}`}>{nameError ?? (source ? "Uzak depo adı düzenleme sırasında korunur." : "Bu bağlantı için kısa bir ad; örneğin origin veya upstream.")}</p>
                </div>

                <div className="repository-field">
                  <label htmlFor={fetchId}>Fetch adresi{fetchHidden && !draft.fetchUrl && <span className="remote-field-badge"><LockKeyhole size={10} aria-hidden="true" />Mevcut adres korunuyor</span>}</label>
                  <input ref={fetchRef} id={fetchId} value={draft.fetchUrl} onChange={(event) => updateDraft({ fetchUrl: event.target.value })} disabled={fieldDisabled} placeholder={fetchHidden ? "Değiştirmek için yeni adres girin" : "https://sunucu/ekip/depo.git"} autoComplete="off" autoCapitalize="none" spellCheck={false} aria-invalid={Boolean(fetchError)} aria-describedby={fetchHintId} />
                  <p id={fetchHintId} className={`repository-field-hint ${fetchError ? "error" : ""}`}>{fetchError ?? (fetchHidden ? "Kayıtlı adres bu alana aktarılmadı. Boş bırakırsanız mevcut adres korunur." : "HTTPS, SSH, SCP adresi veya yerel bir depo yolu kullanabilirsiniz.")}</p>
                  {source && (fetchHidden || source.configuredFetchUrl !== source.fetchUrl) && <CurrentRemoteAddress address={source.fetchUrl} />}
                </div>

                <label className="repository-checkbox remote-push-option">
                  <input type="checkbox" checked={draft.separatePushUrl} onChange={(event) => updateDraft({ separatePushUrl: event.target.checked })} disabled={fieldDisabled} />
                  <span><strong>Push için ayrı adres kullan</strong><small>{draft.separatePushUrl ? "Commit'ler ayrı bir adrese gönderilir." : "Push, Fetch adresini kullanır. Git'in adres yönlendirmeleri korunur."}</small></span>
                </label>

                {draft.separatePushUrl && <div className="repository-field remote-push-field">
                  <label htmlFor={pushId}>Push adresi{pushHidden && !draft.pushUrl && <span className="remote-field-badge"><LockKeyhole size={10} aria-hidden="true" />Mevcut adres korunuyor</span>}</label>
                  <input id={pushId} value={draft.pushUrl} onChange={(event) => updateDraft({ pushUrl: event.target.value })} disabled={fieldDisabled} placeholder={pushHidden ? "Değiştirmek için yeni adres girin" : "git@sunucu:ekip/depo.git"} autoComplete="off" autoCapitalize="none" spellCheck={false} aria-invalid={Boolean(pushError)} aria-describedby={pushHintId} />
                  <p id={pushHintId} className={`repository-field-hint ${pushError ? "error" : ""}`}>{pushError ?? (pushHidden ? "Boş bırakırsanız mevcut ayrı Push adresi korunur. Yukarıdaki seçimi kaldırırsanız Push, Fetch adresini kullanır." : "Yalnızca Push işlemlerinde kullanılacak adres.")}</p>
                  {source?.hasExplicitPushUrl && (pushHidden || source.configuredPushUrl !== source.pushUrl) && <CurrentRemoteAddress address={source.pushUrl} />}
                </div>}

                {source && !draft.separatePushUrl && !source.hasExplicitPushUrl && source.pushUrl !== source.fetchUrl && <div className="remote-inherited-address"><span>Git yönlendirmesiyle kullanılan Push adresi</span><code>{source.pushUrl}</code></div>}

                {!hasChanges && source && !blockedReason && !staleReason && <p className="remote-unchanged-hint"><Check size={12} aria-hidden="true" />Kayıtlı ayarlar gösteriliyor.</p>}
                {feedback && <div className={`repository-form-message ${feedback.kind}`} role={feedback.kind === "error" ? "alert" : "status"}><AlertCircle size={15} aria-hidden="true" /><span>{feedback.message}</span></div>}
                {submitting && <div className="repository-form-message progress" role="status"><LoaderCircle className="spin" size={15} aria-hidden="true" /><span>{refreshing ? "Depo durumu yeniden okunuyor…" : "Uzak depo ayarı kaydediliyor…"}<small>Pencereyi kapatmak işlemi durdurmaz.</small></span></div>}
              </section>
            )}
          </div>
        </div>

        <footer className="repository-dialog-footer remote-manager-footer">
          <span><Link2 size={13} aria-hidden="true" />Kaydetmek ağ bağlantısı kurmaz.</span>
          <button type="button" className="change-action" onClick={closeDialog}>{submitting ? "Pencereyi kapat" : "Kapat"}</button>
          <button type="submit" className="commit-button repository-execute" disabled={!draft || !canMutate || busy || Boolean(formError) || Boolean(staleReason) || !hasChanges}>
            {submitting ? <LoaderCircle className="spin" size={14} aria-hidden="true" /> : source ? <Save size={14} aria-hidden="true" /> : <Plus size={14} aria-hidden="true" />}
            {submitting ? "Kaydediliyor…" : source ? "Değişiklikleri kaydet" : "Uzak depo ekle"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
