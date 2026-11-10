import { AlertCircle, ArrowDown, ArrowUp, Check, ChevronsDown, ChevronsUp, FileCode2, FileDiff, LoaderCircle, Search, TriangleAlert, X } from "lucide-react";
import { type CSSProperties, type KeyboardEvent, type ReactNode, useEffect, useId, useMemo, useRef, useState } from "react";
import { type DiffLine, parseDiffLines } from "./diffLines";
import { type DiffSearchMatch, findDiffHunks, findDiffMatches, stepDiffIndex } from "./diffSearch";
import type { DiffResult } from "./types";
import "./diffView.css";

interface DiffViewProps {
  result: DiffResult | null;
  state: "idle" | "loading" | "ready" | "error";
  error: string | null;
  targetKey?: string;
  partialSelection?: DiffPartialSelection;
}

export interface DiffSelectionRequest {
  selectionToken: string;
  lineIds: number[];
}

export interface DiffPartialSelection {
  staged: boolean;
  selectionToken: string | null;
  selectableLineIds: number[];
  unsupportedReason: string | null;
  busy: boolean;
  onApply: (selection: DiffSelectionRequest) => Promise<void>;
}

function lineDescription(line: DiffLine): string {
  if (line.kind === "add") return `Eklenen satır ${line.newNumber}: `;
  if (line.kind === "remove") return `Silinen satır ${line.oldNumber}: `;
  if (line.kind === "context") return `Eski satır ${line.oldNumber}, yeni satır ${line.newNumber}: `;
  return "";
}

function selectionUnavailableReason(result: DiffResult, partialSelection?: DiffPartialSelection): string | null {
  if (!partialSelection) return null;
  if (partialSelection.unsupportedReason) return partialSelection.unsupportedReason;
  if (result.binary) return "İkili dosyalarda satır veya bölüm seçimi kullanılamıyor.";
  if (result.truncated) return "Kısaltılmış farkta satır veya bölüm seçimi kullanılamıyor.";
  if (!partialSelection.selectionToken) return "Bu fark için satır veya bölüm seçimi kullanılamıyor.";
  return null;
}

function SelectionNotice({ reason }: { reason: string | null }) {
  return reason ? <div className="diff-selection-notice" role="status"><TriangleAlert size={14} /><span>{reason}</span></div> : null;
}

export function DiffView({ result, state, error, targetKey, partialSelection }: DiffViewProps) {
  if (state === "loading") {
    return <div className="panel-message"><LoaderCircle className="spin" size={18} /> Değişiklik okunuyor…</div>;
  }
  if (state === "error") {
    return <div className="panel-message error-message"><AlertCircle size={18} /> <span>{error}</span></div>;
  }
  if (!result) {
    return <div className="panel-message muted-message"><FileDiff size={20} /> Görüntülemek için bir dosya seçin.</div>;
  }
  if (result.binary) {
    return <div className="diff-view"><SelectionNotice reason={selectionUnavailableReason(result, partialSelection)} />
      <div className="panel-message"><FileCode2 size={20} /> Bu dosya ikili; metin farkı gösterilemiyor.</div></div>;
  }
  if (!result.text.trim()) {
    return <div className="diff-view"><SelectionNotice reason={selectionUnavailableReason(result, partialSelection)} />
      <div className="panel-message muted-message"><Check size={18} /> Bu kapsamda gösterilecek fark yok.</div></div>;
  }

  // A new repository/file/layer owns a fresh search, even if its diff is identical.
  return <TextDiffView key={targetKey} result={result} partialSelection={partialSelection} />;
}

interface IndexedMatch extends DiffSearchMatch { index: number }

function highlightedLine(text: string, matches: IndexedMatch[] | undefined, currentIndex: number): ReactNode {
  if (!matches) return text;
  const pieces: ReactNode[] = [];
  let offset = 0;
  for (const match of matches) {
    pieces.push(text.slice(offset, match.start));
    pieces.push(
      <mark
        className={`diff-search-match${match.index === currentIndex ? " is-current" : ""}`}
        data-diff-match={match.index}
        aria-current={match.index === currentIndex ? true : undefined}
        key={match.index}
      >{text.slice(match.start, match.end)}</mark>,
    );
    offset = match.end;
  }
  pieces.push(text.slice(offset));
  return pieces;
}

function scrollDiffTarget(scroller: HTMLDivElement | null, target: HTMLElement | null | undefined, kind: "match" | "hunk", leftInset = 0) {
  if (!scroller || !target) return;
  const viewport = scroller.getBoundingClientRect();
  const bounds = target.getBoundingClientRect();
  const top = scroller.scrollTop + bounds.top - viewport.top - scroller.clientTop;
  const left = scroller.scrollLeft + bounds.left - viewport.left - scroller.clientLeft;
  const margin = 6;
  let nextLeft = scroller.scrollLeft;
  if (kind === "hunk") nextLeft = 0;
  else if (bounds.width + margin * 2 > scroller.clientWidth - leftInset || left - margin < nextLeft + leftInset) nextLeft = left - margin - leftInset;
  else if (left + bounds.width + margin > nextLeft + scroller.clientWidth) nextLeft = left + bounds.width + margin - scroller.clientWidth;
  const nextTop = kind === "hunk" ? top : top - (scroller.clientHeight - bounds.height) / 2;

  // scrollIntoView also moves overflow-hidden panel ancestors. Only this code
  // viewport owns navigation; the search toolbar and surrounding grid stay put.
  scroller.scrollTo({
    top: Math.max(0, Math.min(nextTop, scroller.scrollHeight - scroller.clientHeight)),
    left: Math.max(0, Math.min(nextLeft, scroller.scrollWidth - scroller.clientWidth)),
    behavior: "auto",
  });
}

function SelectionCheckbox({ label, checked, mixed = false, disabled, onChange }: {
  label: string;
  checked: boolean;
  mixed?: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  const checkboxRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (checkboxRef.current) checkboxRef.current.indeterminate = mixed;
  }, [mixed]);
  return <input className="diff-selection-checkbox" ref={checkboxRef} type="checkbox" checked={checked}
    aria-label={label} aria-checked={mixed ? "mixed" : checked} disabled={disabled}
    onChange={(event) => onChange(event.target.checked)} />;
}

interface SelectedLines {
  result: DiffResult;
  selectionToken: string | null;
  staged: boolean;
  lineIds: Set<number>;
}

function TextDiffView({ result, partialSelection }: { result: DiffResult; partialSelection?: DiffPartialSelection }) {
  const lines = useMemo(() => parseDiffLines(result.text), [result.text]);
  const [query, setQuery] = useState("");
  const { matches, limited } = useMemo(() => findDiffMatches(lines, query), [lines, query]);
  const hunks = useMemo(() => findDiffHunks(lines), [lines]);
  const [selectedMatch, setSelectedMatch] = useState<{ matches: DiffSearchMatch[]; index: number } | null>(null);
  const [selectedHunk, setSelectedHunk] = useState<{ lines: DiffLine[]; index: number } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const codeRef = useRef<HTMLDivElement>(null);
  const searchDescriptionId = useId();
  const [selection, setSelection] = useState<SelectedLines | null>(null);
  const [submittingSelection, setSubmittingSelection] = useState(false);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const selectionInFlight = useRef(false);
  const selectionToken = partialSelection?.selectionToken ?? null;
  const staged = partialSelection?.staged ?? false;
  const unavailableReason = selectionUnavailableReason(result, partialSelection);
  const selectableIds = useMemo(() => {
    if (!partialSelection || unavailableReason) return new Set<number>();
    // Parser output retains physical patch row positions. Source text is never
    // used as an identity or turned into a patch in this view.
    return new Set(partialSelection.selectableLineIds.filter((id) => Number.isInteger(id)
      && (lines[id]?.kind === "add" || lines[id]?.kind === "remove")));
  }, [lines, partialSelection?.selectableLineIds, unavailableReason]);
  const selectedIds = selection?.result === result && selection.selectionToken === selectionToken && selection.staged === staged
    ? new Set([...selection.lineIds].filter((id) => selectableIds.has(id))) : new Set<number>();
  const hunkSelections = useMemo(() => {
    const grouped = new Map<number, number[]>();
    let currentHunk = -1;
    lines.forEach((line, index) => {
      if (line.kind === "hunk") { currentHunk = index; grouped.set(index, []); }
      else if (currentHunk >= 0 && selectableIds.has(index)) grouped.get(currentHunk)?.push(index);
    });
    return grouped;
  }, [lines, selectableIds]);
  const selectionBusy = Boolean(partialSelection?.busy || submittingSelection);
  const actionLabel = staged ? "Unstage" : "Stage";
  const selectedActionLabel = staged ? "Seçili satırları Unstage yap" : "Seçili satırları Stage'e al";
  const hasSelectionGutter = selectableIds.size > 0;
  // New query/results immediately discard old offsets before the next paint.
  const matchIndex = matches.length === 0 ? -1 : selectedMatch?.matches === matches ? selectedMatch.index : 0;
  const hunkIndex = selectedHunk?.lines === lines ? selectedHunk.index : -1;
  const totalLabel = `${matches.length}${limited ? "+" : ""}`;
  const matchesByLine = useMemo(() => {
    const grouped = new Map<number, IndexedMatch[]>();
    matches.forEach((match, index) => {
      const group = grouped.get(match.lineIndex) ?? [];
      group.push({ ...match, index });
      grouped.set(match.lineIndex, group);
    });
    return grouped;
  }, [matches]);

  useEffect(() => {
    if (matches.length) {
      scrollDiffTarget(scrollRef.current, codeRef.current?.querySelector<HTMLElement>('[data-diff-match="0"]'), "match", hasSelectionGutter ? 24 : 0);
    }
  }, [matches]);

  useEffect(() => {
    setSelection(null);
    setSelectionError(null);
  }, [result, selectionToken, staged]);

  function selectLines(lineIds: number[], checked: boolean) {
    if (!partialSelection || unavailableReason || selectionBusy || selectionInFlight.current) return;
    const next = new Set(selectedIds);
    for (const id of lineIds) {
      if (!selectableIds.has(id)) continue;
      if (checked) next.add(id);
      else next.delete(id);
    }
    setSelection({ result, selectionToken, staged, lineIds: next });
  }

  function clearSelection() {
    if (selectionBusy || selectionInFlight.current) return;
    setSelection(null);
  }

  async function applySelection(lineIds: number[]) {
    if (!partialSelection || !selectionToken || unavailableReason || selectionBusy || selectionInFlight.current) return;
    const eligibleIds = [...new Set(lineIds)].filter((id) => selectableIds.has(id)).sort((left, right) => left - right);
    if (!eligibleIds.length) return;
    // The ref closes the same-event double-submit window before React can paint
    // disabled controls; the parent also serializes repository writes.
    selectionInFlight.current = true;
    setSubmittingSelection(true);
    setSelectionError(null);
    try {
      await partialSelection.onApply({ selectionToken, lineIds: eligibleIds });
      setSelection(null);
    } catch (applyError) {
      setSelectionError(applyError instanceof Error ? applyError.message : String(applyError));
    } finally {
      selectionInFlight.current = false;
      setSubmittingSelection(false);
    }
  }

  function moveMatch(direction: 1 | -1) {
    const index = stepDiffIndex(matchIndex, matches.length, direction);
    if (index < 0) return;
    setSelectedMatch({ matches, index });
    scrollDiffTarget(scrollRef.current, codeRef.current?.querySelector<HTMLElement>(`[data-diff-match="${index}"]`), "match", hasSelectionGutter ? 24 : 0);
  }

  function moveHunk(direction: 1 | -1) {
    const index = stepDiffIndex(hunkIndex, hunks.length, direction);
    if (index < 0) return;
    setSelectedHunk({ lines, index });
    scrollDiffTarget(scrollRef.current, codeRef.current?.querySelector<HTMLElement>(`[data-diff-line="${hunks[index]}"]`), "hunk");
  }

  function clearSearch() {
    setQuery("");
    setSelectedMatch(null);
    searchRef.current?.focus({ preventScroll: true });
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.nativeEvent.isComposing) return;
    if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "f") {
      event.preventDefault();
      event.stopPropagation();
      searchRef.current?.focus({ preventScroll: true });
      searchRef.current?.select();
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === "F3" || (event.key === "Enter" && event.target === searchRef.current)) {
      event.preventDefault();
      event.stopPropagation();
      if (query) moveMatch(event.shiftKey ? -1 : 1);
      else searchRef.current?.focus({ preventScroll: true });
    } else if (event.key === "Escape" && event.target === searchRef.current) {
      event.preventDefault();
      event.stopPropagation();
      clearSearch();
    }
  }

  const numberDigits = lines.reduce(
    (digits, line) => Math.max(digits, String(line.oldNumber ?? "").length, String(line.newNumber ?? "").length),
    3,
  );
  const codeStyle = { "--diff-line-number-width": `${numberDigits + 2}ch` } as CSSProperties;

  return (
    <div className="diff-view" role="region" aria-label="Dosya farkı" tabIndex={0} onKeyDown={handleKeyDown}>
      <div className="diff-view-toolbar" role="group" aria-label="Farkta arama ve gezinme">
        <div className="diff-search-controls">
          <label className="diff-search-field">
            <Search size={13} aria-hidden="true" />
            <input ref={searchRef} type="text" value={query} onChange={(event) => setQuery(event.target.value)}
              aria-label="Farkta ara" aria-describedby={searchDescriptionId} aria-keyshortcuts="Control+f Meta+f"
              placeholder="Farkta ara…" autoComplete="off" spellCheck={false} />
          </label>
          <span className="diff-search-count" role="status" aria-live="polite" aria-atomic="true"
            aria-label={!query ? "Arama metni girilmedi" : matches.length ? `Eşleşme ${matchIndex + 1}, ${limited ? "en az" : "toplam"} ${matches.length}` : "Eşleşme yok"}
            title={!query ? "Aramak için metin girin" : matches.length ? `${matchIndex + 1} / ${totalLabel} eşleşme` : "Eşleşme yok"}>
            {matchIndex + 1} / {totalLabel}
          </span>
          <button type="button" className="diff-nav-button" onClick={() => moveMatch(-1)} disabled={!matches.length}
            aria-label="Önceki eşleşme" title="Önceki eşleşme (Shift+Enter / Shift+F3)"><ArrowUp size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={() => moveMatch(1)} disabled={!matches.length}
            aria-label="Sonraki eşleşme" title="Sonraki eşleşme (Enter / F3)"><ArrowDown size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={clearSearch} disabled={!query}
            aria-label="Fark aramasını temizle" title="Aramayı temizle (Escape)"><X size={13} /></button>
        </div>
        <div className="diff-hunk-controls" role="group" aria-label="Değişiklik bölümleri">
          <span className="diff-hunk-count" title={`${hunks.length} değişiklik bölümü`}>
            Bölüm {hunkIndex < 0 ? "–" : hunkIndex + 1} / {hunks.length}
          </span>
          <button type="button" className="diff-nav-button" onClick={() => moveHunk(-1)} disabled={!hunks.length}
            aria-label="Önceki değişiklik bölümü" title="Önceki değişiklik bölümü"><ChevronsUp size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={() => moveHunk(1)} disabled={!hunks.length}
            aria-label="Sonraki değişiklik bölümü" title="Sonraki değişiklik bölümü"><ChevronsDown size={14} /></button>
        </div>
        <span id={searchDescriptionId} className="diff-view-line-description">
          Görünen fark metninde büyük/küçük harf duyarsız, düz metin araması. Enter veya F3: sonraki; Shift ile önceki eşleşme.
        </span>
      </div>
      {partialSelection && (
        <div className="diff-selection-toolbar" role="group" aria-label="Satır seçimi">
          <span className="diff-selection-count" role="status" aria-live="polite" aria-atomic="true">{selectedIds.size} satır seçili</span>
          <div className="diff-selection-actions">
            <button type="button" className="diff-selection-button" onClick={clearSelection}
              disabled={selectionBusy || !selectedIds.size} aria-label="Satır seçimini temizle">Temizle</button>
            <button type="button" className="diff-selection-button diff-selection-apply" onClick={() => applySelection([...selectedIds])}
              disabled={selectionBusy || !selectedIds.size || Boolean(unavailableReason)} aria-label={selectedActionLabel}>
              {submittingSelection && <LoaderCircle size={12} className="spin" aria-hidden="true" />}{actionLabel}
            </button>
          </div>
        </div>
      )}
      <SelectionNotice reason={unavailableReason} />
      {selectionError && <div className="diff-selection-error" role="alert"><AlertCircle size={14} /><span>{selectionError}</span></div>}
      {result.truncated && (
        <div className="diff-truncated"><TriangleAlert size={14} /> <span>Fark çıktısı kısaltıldı. Arama ve bölüm gezintisi yalnızca gösterilen kısımda.</span></div>
      )}
      {limited && (
        <div className="diff-search-limit" role="status"><TriangleAlert size={14} /> <span>İlk {matches.length.toLocaleString("tr-TR")} eşleşme gösteriliyor. Aramayı daraltın.</span></div>
      )}
      <div className="diff-scroll" tabIndex={0} aria-label="Fark satırları" ref={scrollRef}>
        <div className={`diff-view-code${hasSelectionGutter ? " has-selection" : ""}`} style={codeStyle} ref={codeRef}>
          {lines.map((line, index) => {
            const hunkLineIds = hunkSelections.get(index) ?? [];
            const selectedHunkLineCount = hunkLineIds.filter((id) => selectedIds.has(id)).length;
            const hunkNumber = line.kind === "hunk" ? hunks.indexOf(index) + 1 : 0;
            return (
            <div className={`diff-view-row diff-view-${line.kind}${index === hunks[hunkIndex] ? " is-current-hunk" : ""}${selectedIds.has(index) ? " is-selected-line" : ""}`} key={index} data-diff-line={index}>
              <span className="diff-view-line-description">{lineDescription(line)}</span>
              {hasSelectionGutter && <span className="diff-selection-cell">
                {selectableIds.has(index) ? (
                  <SelectionCheckbox label={`${line.kind === "add" ? "Eklenen" : "Silinen"} satır ${line.newNumber ?? line.oldNumber} seç`}
                    checked={selectedIds.has(index)} disabled={selectionBusy} onChange={(checked) => selectLines([index], checked)} />
                ) : hunkLineIds.length > 0 ? (
                  <SelectionCheckbox label={`${hunkNumber}. bölümün değişen satırlarını seç`}
                    checked={selectedHunkLineCount === hunkLineIds.length} mixed={selectedHunkLineCount > 0 && selectedHunkLineCount < hunkLineIds.length}
                    disabled={selectionBusy} onChange={(checked) => selectLines(hunkLineIds, checked)} />
                ) : null}
              </span>}
              <span className="diff-view-number diff-view-old-number" aria-hidden="true">{line.oldNumber}</span>
              <span className="diff-view-number diff-view-new-number" aria-hidden="true">{line.newNumber}</span>
              <span className="diff-view-marker" aria-hidden="true">{line.kind === "add" ? "+" : line.kind === "remove" ? "−" : ""}</span>
              {hunkLineIds.length > 0 && <span className="diff-hunk-selection-action">
                <button type="button" className="diff-selection-button" onClick={() => applySelection(hunkLineIds)} disabled={selectionBusy}
                  aria-label={`${hunkNumber}. bölümü ${actionLabel} yap`} title={`${hunkLineIds.length} değişen satır: ${actionLabel}`}>
                  {actionLabel} bölüm
                </button>
              </span>}
              <code>{highlightedLine(line.text, matchesByLine.get(index), matchIndex)}</code>
            </div>
          );})}
        </div>
      </div>
    </div>
  );
}
