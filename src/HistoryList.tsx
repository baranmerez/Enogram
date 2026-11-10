import { type CSSProperties, type KeyboardEvent, useEffect, useMemo, useRef } from "react";
import { buildHistoryGraph, compactHistoryRowHeight, historyContinuationHeight, historyRowHeight, parseCommitRefs } from "./historyGraph";
import type { CommitEntry } from "./types";
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

const dateFormatter = new Intl.DateTimeFormat("tr-TR", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value || "Tarih bilinmiyor" : dateFormatter.format(date);
}

function commitTitle(commit: CommitEntry): string {
  return [
    commit.subject,
    commit.oid,
    `${commit.author} · ${formatDate(commit.date)}`,
    commit.refs,
    commit.parents.length ? `Üst commit: ${commit.parents.join(", ")}` : "Kök commit",
  ].filter(Boolean).join("\n");
}

export function HistoryList({ commits, selectedOid, onSelect, disabled = false, density = "comfortable", matchingOids, revealOid }: HistoryListProps) {
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
        <ol className="history-graph-rows" aria-label="Commit geçmişi">
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
                  title={commitTitle(commit)}
                >
                  <span className="history-graph-spacer" aria-hidden="true" />
                  <span className="history-graph-commit-main">
                    <span className="history-graph-subject">{commit.subject || "(Başlıksız commit)"}</span>
                    <span className="history-graph-meta">
                      <code>{commit.oid.slice(0, 8)}</code>
                      {refs[0] && <span className={`history-graph-ref ${refs[0].kind}`} title={commit.refs}>{refs[0].label}</span>}
                      {refs.length > 1 && <span className="history-graph-ref-more" title={commit.refs} aria-label={`Diğer referanslar: ${refs.slice(1).map((ref) => ref.label).join(", ")}`}>+{refs.length - 1}</span>}
                      <span className="history-graph-author">{commit.author}</span>
                    </span>
                  </span>
                  <span className="history-graph-date">{formatDate(commit.date)}</span>
                  <span className="history-graph-sr-only">
                    {commit.parents.length ? `Üst commit: ${commit.parents.join(", ")}` : "Kök commit"}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
        {graph.boundaries.length > 0 && (
          <div className="history-graph-continuation" title={`${boundaryCount} üst commit bu görünümün dışında: ${[...new Set(graph.boundaries.map((boundary) => boundary.parentOid))].join(", ")}`}>
            <span>Geçmiş bu listenin dışında devam ediyor</span>
            <span className="history-graph-sr-only">{boundaryCount} üst commit görüntülenen geçmişin dışında.</span>
          </div>
        )}
      </div>
    </div>
  );
}
