import { type CSSProperties, type KeyboardEvent, useEffect, useMemo, useRef } from "react";
import { buildHistoryGraph, compactHistoryRowHeight, historyContinuationHeight, historyRowHeight, parseCommitRefs } from "./historyGraph";
import { intlLocale } from "./i18n.ts";
import { diffHistoryMessages } from "./locales/diffHistory.ts";
import type { CommitEntry } from "./types";
import { useI18n } from "./useI18n";
import "./historyGraph.css";

interface HistoryListProps {
  commits: CommitEntry[];
  selectedOid: string | null;
  onSelect: (commit: CommitEntry) => void;
  disabled?: boolean;
  density?: "compact" | "comfortable";
  matchingOids?: ReadonlySet<string>;
  revealOid?: string | null;
}

type Translate = ReturnType<typeof useI18n>["t"];

function formatDate(value: string, dateFormatter: Intl.DateTimeFormat, unknownDate: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value || unknownDate : dateFormatter.format(date);
}

function parentDescription(commit: CommitEntry, t: Translate): string {
  return commit.parents.length ? t(commit.parents.length === 1 ? "history.parent" : "history.parents", { parents: commit.parents.join(", ") }) : t("history.rootCommit");
}

function commitTitle(commit: CommitEntry, dateFormatter: Intl.DateTimeFormat, t: Translate): string {
  return [
    commit.subject,
    commit.oid,
    `${commit.author} · ${formatDate(commit.date, dateFormatter, t("history.unknownDate"))}`,
    commit.refs,
    parentDescription(commit, t),
  ].filter(Boolean).join("\n");
}

export function HistoryList({ commits, selectedOid, onSelect, disabled = false, density = "comfortable", matchingOids, revealOid }: HistoryListProps) {
  const { locale, t } = useI18n(diffHistoryMessages);
  const dateFormatter = useMemo(() => new Intl.DateTimeFormat(intlLocale(locale), { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" }), [locale]);
  const numberFormatter = useMemo(() => new Intl.NumberFormat(intlLocale(locale)), [locale]);
  const rowHeight = density === "compact" ? compactHistoryRowHeight : historyRowHeight;
  const graph = useMemo(() => buildHistoryGraph(commits, rowHeight), [commits, rowHeight]);
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const style = {
    "--history-graph-width": `${graph.width}px`,
    "--history-row-height": `${rowHeight}px`,
    "--history-continuation-height": `${historyContinuationHeight}px`,
  } as CSSProperties;
  const boundaryCount = new Set(graph.boundaries.map((boundary) => boundary.parentOid)).size;

  useEffect(() => {
    if (!revealOid) return;
    const index = commits.findIndex((commit) => commit.oid === revealOid);
    // Finding a commit keeps the search field focused and all parent links visible.
    buttons.current[index]?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [commits, revealOid, rowHeight]);

  const moveFocus = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (disabled || event.altKey || event.ctrlKey || event.metaKey) return;
    let next = index;
    if (event.key === "ArrowDown") next = Math.min(index + 1, commits.length - 1);
    else if (event.key === "ArrowUp") next = Math.max(index - 1, 0);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = commits.length - 1;
    else return;
    event.preventDefault();
    buttons.current[next]?.focus({ preventScroll: true });
    buttons.current[next]?.scrollIntoView({ block: "nearest", inline: "nearest" });
  };

  return (
    <div className="history-graph-list" style={style} data-density={density}>
      <div className="history-graph-content">
        <svg className="history-graph-svg" width={graph.width} height={graph.height} aria-hidden="true" focusable="false">
          <g className="history-graph-edges">
            {graph.edges.map((edge) => (
              <path key={edge.id} d={edge.path} stroke={edge.color} data-child={edge.childOid} data-parent={edge.parentOid} />
            ))}
          </g>
          <g className="history-graph-boundaries">
            {graph.boundaries.map((boundary) => (
              <g key={boundary.edgeId} stroke={boundary.color}>
                <path d={`M ${boundary.x} ${boundary.yStart} V ${boundary.yEnd}`} strokeDasharray="3 4" />
                <path d={`M ${boundary.x - 3} ${boundary.yEnd - 3} L ${boundary.x} ${boundary.yEnd} L ${boundary.x + 3} ${boundary.yEnd - 3}`} />
              </g>
            ))}
          </g>
          <g className="history-graph-nodes">
            {graph.nodes.map((node) => (
              <g key={node.oid} className={node.oid === selectedOid ? "history-graph-node selected" : "history-graph-node"}>
                <circle className="history-graph-node-halo" cx={node.x} cy={node.y} r={7} />
                <circle className="history-graph-node-ring" cx={node.x} cy={node.y} r={node.isMerge ? 5 : 4} stroke={node.color} />
                {(node.isMerge || node.oid === selectedOid) && <circle cx={node.x} cy={node.y} r={2} fill={node.color} />}
              </g>
            ))}
          </g>
        </svg>
        <ol className="history-graph-rows" aria-label={t("history.list")}>
          {commits.map((commit, index) => {
            const refs = parseCommitRefs(commit.refs);
            const selected = commit.oid === selectedOid;
            const searchMatch = matchingOids?.has(commit.oid) ?? false;
            return (
              <li key={commit.oid}>
                <button
                  className={`history-graph-commit${selected ? " selected" : ""}${searchMatch ? " search-match" : ""}`}
                  type="button"
                  ref={(button) => { buttons.current[index] = button; }}
                  disabled={disabled}
                  aria-pressed={selected}
                  data-search-match={searchMatch ? "true" : undefined}
                  onClick={() => onSelect(commit)}
                  onKeyDown={(event) => moveFocus(event, index)}
                  title={commitTitle(commit, dateFormatter, t)}
                >
                  <span className="history-graph-spacer" aria-hidden="true" />
                  <span className="history-graph-commit-main">
                    <span className="history-graph-subject">{commit.subject || t("history.untitledCommit")}</span>
                    <span className="history-graph-meta">
                      <code>{commit.oid.slice(0, 8)}</code>
                      {refs[0] && <span className={`history-graph-ref ${refs[0].kind}`} title={commit.refs}>{refs[0].label}</span>}
                      {refs.length > 1 && <span className="history-graph-ref-more" title={commit.refs} aria-label={t("history.otherRefs", { refs: refs.slice(1).map((ref) => ref.label).join(", ") })}>+{numberFormatter.format(refs.length - 1)}</span>}
                      <span className="history-graph-author">{commit.author}</span>
                    </span>
                  </span>
                  <span className="history-graph-date">{formatDate(commit.date, dateFormatter, t("history.unknownDate"))}</span>
                  <span className="history-graph-sr-only">
                    {parentDescription(commit, t)}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
        {graph.boundaries.length > 0 && (
          <div className="history-graph-continuation" title={t(boundaryCount === 1 ? "history.boundaryTitleOne" : "history.boundaryTitleMany", { count: numberFormatter.format(boundaryCount), parents: [...new Set(graph.boundaries.map((boundary) => boundary.parentOid))].join(", ") })}>
            <span>{t("history.continuation")}</span>
            <span className="history-graph-sr-only">{t(boundaryCount === 1 ? "history.boundaryDescriptionOne" : "history.boundaryDescriptionMany", { count: numberFormatter.format(boundaryCount) })}</span>
          </div>
        )}
      </div>
    </div>
  );
}
