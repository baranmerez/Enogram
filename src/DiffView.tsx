import { AlertCircle, ArrowDown, ArrowUp, Check, ChevronsDown, ChevronsUp, FileCode2, FileDiff, LoaderCircle, Search, TriangleAlert, X } from "lucide-react";
import { type CSSProperties, type KeyboardEvent, type ReactNode, useEffect, useId, useMemo, useRef, useState } from "react";
import { type DiffLine, parseDiffLines } from "./diffLines";
import { type DiffSearchMatch, findDiffHunks, findDiffMatches, stepDiffIndex } from "./diffSearch";
import { intlLocale, type Locale } from "./i18n.ts";
import { localizeBackendText } from "./localizeBackendText.ts";
import { diffHistoryMessages } from "./locales/diffHistory.ts";
import type { DiffResult } from "./types";
import { useI18n } from "./useI18n";
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

type Translate = ReturnType<typeof useI18n>["t"];

function lineDescription(line: DiffLine, t: Translate, numberFormatter: Intl.NumberFormat): string {
  if (line.kind === "add") return t("diff.addedLineDescription", { line: numberFormatter.format(line.newNumber!) });
  if (line.kind === "remove") return t("diff.removedLineDescription", { line: numberFormatter.format(line.oldNumber!) });
  if (line.kind === "context") return t("diff.contextLineDescription", { oldLine: numberFormatter.format(line.oldNumber!), newLine: numberFormatter.format(line.newNumber!) });
  return "";
}

function selectionUnavailableReason(result: DiffResult, t: Translate, locale: Locale, partialSelection?: DiffPartialSelection): string | null {
  if (!partialSelection) return null;
  if (partialSelection.unsupportedReason) return localizeBackendText(partialSelection.unsupportedReason, locale);
  if (result.binary) return t("diff.selectionUnavailableBinary");
  if (result.truncated) return t("diff.selectionUnavailableTruncated");
  if (!partialSelection.selectionToken) return t("diff.selectionUnavailable");
  return null;
}

function SelectionNotice({ reason }: { reason: string | null }) {
  return reason ? <div className="diff-selection-notice" role="status"><TriangleAlert size={14} /><span>{reason}</span></div> : null;
}

export function DiffView({ result, state, error, targetKey, partialSelection }: DiffViewProps) {
  const { locale, t } = useI18n(diffHistoryMessages);
  if (state === "loading") {
    return <div className="panel-message"><LoaderCircle className="spin" size={18} /> {t("diff.loading")}</div>;
  }
  if (state === "error") {
    return <div className="panel-message error-message"><AlertCircle size={18} /> <span>{error}</span></div>;
  }
  if (!result) {
    return <div className="panel-message muted-message"><FileDiff size={20} /> {t("diff.selectFile")}</div>;
  }
  if (result.binary) {
    return <div className="diff-view"><SelectionNotice reason={selectionUnavailableReason(result, t, locale, partialSelection)} />
      <div className="panel-message"><FileCode2 size={20} /> {t("diff.binary")}</div></div>;
  }
  if (!result.text.trim()) {
    return <div className="diff-view"><SelectionNotice reason={selectionUnavailableReason(result, t, locale, partialSelection)} />
      <div className="panel-message muted-message"><Check size={18} /> {t("diff.empty")}</div></div>;
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
  const { locale, t } = useI18n(diffHistoryMessages);
  const numberFormatter = useMemo(() => new Intl.NumberFormat(intlLocale(locale)), [locale]);
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
  const unavailableReason = selectionUnavailableReason(result, t, locale, partialSelection);
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
  const actionLabel = t(staged ? "diff.unstage" : "diff.stage");
  const selectedActionLabel = t(staged ? "diff.unstageSelected" : "diff.stageSelected");
  const hasSelectionGutter = selectableIds.size > 0;
  // New query/results immediately discard old offsets before the next paint.
  const matchIndex = matches.length === 0 ? -1 : selectedMatch?.matches === matches ? selectedMatch.index : 0;
  const hunkIndex = selectedHunk?.lines === lines ? selectedHunk.index : -1;
  const currentMatchLabel = numberFormatter.format(matchIndex + 1);
  const totalLabel = `${numberFormatter.format(matches.length)}${limited ? "+" : ""}`;
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
    <div className="diff-view" role="region" aria-label={t("diff.region")} tabIndex={0} onKeyDown={handleKeyDown}>
      <div className="diff-view-toolbar" role="group" aria-label={t("diff.searchToolbar")}>
        <div className="diff-search-controls">
          <label className="diff-search-field">
            <Search size={13} aria-hidden="true" />
            <input ref={searchRef} type="text" value={query} onChange={(event) => setQuery(event.target.value)}
              aria-label={t("diff.search")} aria-describedby={searchDescriptionId} aria-keyshortcuts="Control+f Meta+f"
              placeholder={t("diff.searchPlaceholder")} autoComplete="off" spellCheck={false} />
          </label>
          <span className="diff-search-count" role="status" aria-live="polite" aria-atomic="true"
            aria-label={!query ? t("diff.searchEmpty") : matches.length ? t(limited ? "diff.matchPositionLimited" : "diff.matchPosition", { current: currentMatchLabel, total: numberFormatter.format(matches.length) }) : t("diff.noMatches")}
            title={!query ? t("diff.searchPrompt") : matches.length ? t("diff.matchCount", { current: currentMatchLabel, total: totalLabel }) : t("diff.noMatches")}>
            {currentMatchLabel} / {totalLabel}
          </span>
          <button type="button" className="diff-nav-button" onClick={() => moveMatch(-1)} disabled={!matches.length}
            aria-label={t("diff.previousMatch")} title={t("diff.previousMatchHint")}><ArrowUp size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={() => moveMatch(1)} disabled={!matches.length}
            aria-label={t("diff.nextMatch")} title={t("diff.nextMatchHint")}><ArrowDown size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={clearSearch} disabled={!query}
            aria-label={t("diff.clearSearch")} title={t("diff.clearSearchHint")}><X size={13} /></button>
        </div>
        <div className="diff-hunk-controls" role="group" aria-label={t("diff.hunks")}>
          <span className="diff-hunk-count" title={t(hunks.length === 1 ? "diff.hunkCountOne" : "diff.hunkCountMany", { count: numberFormatter.format(hunks.length) })}>
            {t("diff.hunkPosition", { current: hunkIndex < 0 ? "–" : numberFormatter.format(hunkIndex + 1), total: numberFormatter.format(hunks.length) })}
          </span>
          <button type="button" className="diff-nav-button" onClick={() => moveHunk(-1)} disabled={!hunks.length}
            aria-label={t("diff.previousHunk")} title={t("diff.previousHunk")}><ChevronsUp size={14} /></button>
          <button type="button" className="diff-nav-button" onClick={() => moveHunk(1)} disabled={!hunks.length}
            aria-label={t("diff.nextHunk")} title={t("diff.nextHunk")}><ChevronsDown size={14} /></button>
        </div>
        <span id={searchDescriptionId} className="diff-view-line-description">
          {t("diff.searchDescription")}
        </span>
      </div>
      {partialSelection && (
        <div className="diff-selection-toolbar" role="group" aria-label={t("diff.lineSelection")}>
          <span className="diff-selection-count" role="status" aria-live="polite" aria-atomic="true">{t(selectedIds.size === 1 ? "diff.selectedLineCountOne" : "diff.selectedLineCountMany", { count: numberFormatter.format(selectedIds.size) })}</span>
          <div className="diff-selection-actions">
            <button type="button" className="diff-selection-button" onClick={clearSelection}
              disabled={selectionBusy || !selectedIds.size} aria-label={t("diff.clearSelection")}>{t("diff.clear")}</button>
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
        <div className="diff-truncated"><TriangleAlert size={14} /> <span>{t("diff.truncated")}</span></div>
      )}
      {limited && (
        <div className="diff-search-limit" role="status"><TriangleAlert size={14} /> <span>{t("diff.searchLimit", { count: numberFormatter.format(matches.length) })}</span></div>
      )}
      <div className="diff-scroll" tabIndex={0} aria-label={t("diff.lines")} ref={scrollRef}>
        <div className={`diff-view-code${hasSelectionGutter ? " has-selection" : ""}`} style={codeStyle} ref={codeRef}>
          {lines.map((line, index) => {
            const hunkLineIds = hunkSelections.get(index) ?? [];
            const selectedHunkLineCount = hunkLineIds.filter((id) => selectedIds.has(id)).length;
            const hunkNumber = line.kind === "hunk" ? hunks.indexOf(index) + 1 : 0;
            return (
            <div className={`diff-view-row diff-view-${line.kind}${index === hunks[hunkIndex] ? " is-current-hunk" : ""}${selectedIds.has(index) ? " is-selected-line" : ""}`} key={index} data-diff-line={index}>
              <span className="diff-view-line-description">{lineDescription(line, t, numberFormatter)}</span>
              {hasSelectionGutter && <span className="diff-selection-cell">
                {selectableIds.has(index) ? (
                  <SelectionCheckbox label={t(line.kind === "add" ? "diff.selectAddedLine" : "diff.selectRemovedLine", { line: numberFormatter.format((line.newNumber ?? line.oldNumber)!) })}
                    checked={selectedIds.has(index)} disabled={selectionBusy} onChange={(checked) => selectLines([index], checked)} />
                ) : hunkLineIds.length > 0 ? (
                  <SelectionCheckbox label={t("diff.selectHunk", { hunk: numberFormatter.format(hunkNumber) })}
                    checked={selectedHunkLineCount === hunkLineIds.length} mixed={selectedHunkLineCount > 0 && selectedHunkLineCount < hunkLineIds.length}
                    disabled={selectionBusy} onChange={(checked) => selectLines(hunkLineIds, checked)} />
                ) : null}
              </span>}
              <span className="diff-view-number diff-view-old-number" aria-hidden="true">{line.oldNumber}</span>
              <span className="diff-view-number diff-view-new-number" aria-hidden="true">{line.newNumber}</span>
              <span className="diff-view-marker" aria-hidden="true">{line.kind === "add" ? "+" : line.kind === "remove" ? "−" : ""}</span>
              {hunkLineIds.length > 0 && <span className="diff-hunk-selection-action">
                <button type="button" className="diff-selection-button" onClick={() => applySelection(hunkLineIds)} disabled={selectionBusy}
                  aria-label={t("diff.applyHunk", { hunk: numberFormatter.format(hunkNumber), action: actionLabel })}
                  title={t(hunkLineIds.length === 1 ? "diff.hunkActionTitleOne" : "diff.hunkActionTitleMany", { count: numberFormatter.format(hunkLineIds.length), action: actionLabel })}>
                  {t("diff.hunkAction", { action: actionLabel })}
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
