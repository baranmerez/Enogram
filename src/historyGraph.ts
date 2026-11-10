/** Fixed rows keep the SVG endpoints aligned with the commit buttons at every zoom level. */
export const historyRowHeight = 50;
export const compactHistoryRowHeight = 36;
export const historyContinuationHeight = 34;

const laneGap = 20;
const graphPadding = 16;
const laneColors = [
  "var(--graph-1, #79b8ff)", "var(--graph-2, #d2a8ff)",
  "var(--graph-3, #7bd9aa)", "var(--graph-4, #f2c778)",
  "var(--graph-5, #f594ad)", "var(--graph-6, #74d3de)",
  "var(--graph-7, #b7cf76)", "var(--graph-8, #efa57c)",
];

interface GraphCommit {
  oid: string;
  parents: readonly string[];
}

export interface HistoryGraphNode {
  oid: string;
  row: number;
  lane: number;
  x: number;
  y: number;
  color: string;
  isMerge: boolean;
}

export interface HistoryGraphEdge {
  id: string;
  childOid: string;
  parentOid: string;
  childRow: number;
  parentRow: number | null;
  lane: number;
  color: string;
  path: string;
}

export interface HistoryGraphBoundary {
  edgeId: string;
  parentOid: string;
  lane: number;
  color: string;
  x: number;
  yStart: number;
  yEnd: number;
}

export interface HistoryGraph {
  nodes: HistoryGraphNode[];
  edges: HistoryGraphEdge[];
  boundaries: HistoryGraphBoundary[];
  laneCount: number;
  width: number;
  height: number;
  rowsHeight: number;
}

export interface CommitRef {
  label: string;
  kind: "head" | "tag" | "branch";
}

interface PendingEdge {
  id: string;
  childOid: string;
  parentOid: string;
  childRow: number;
  parentRow: number | null;
  lowerRow: number | null;
  lane: number;
}

function laneX(lane: number): number {
  return graphPadding + lane * laneGap;
}

function laneColor(lane: number): string {
  return laneColors[lane % laneColors.length];
}

function firstFreeLane(lanes: (PendingEdge | null)[]): number {
  const free = lanes.indexOf(null);
  return free === -1 ? lanes.length : free;
}

/** Git's decoration separator includes a space; a comma alone can be part of a ref name. */
export function parseCommitRefs(refs: string): CommitRef[] {
  return refs.split(/,\s+/).map((ref) => ref.trim()).filter(Boolean).map((ref) => {
    if (ref === "HEAD" || ref.startsWith("HEAD -> ")) {
      return { label: ref.replace("HEAD -> ", "HEAD → "), kind: "head" };
    }
    if (ref.startsWith("tag: ")) return { label: ref, kind: "tag" };
    return { label: ref, kind: "branch" };
  });
}

function edgePath(edge: PendingEdge, nodes: HistoryGraphNode[], rowsHeight: number, rowHeight: number): string {
  const child = nodes[edge.childRow];
  const parent = edge.parentRow === null ? null : nodes[edge.parentRow];
  const direction = parent && parent.row < child.row ? -1 : 1;
  const routeX = laneX(edge.lane);
  const exitY = (child.row + (direction === 1 ? 1 : 0)) * rowHeight;
  const entryY = parent
    ? (parent.row + (direction === 1 ? 0 : 1)) * rowHeight
    : rowsHeight;
  const curve = rowHeight / 4 * direction;
  const segments = [`M ${child.x} ${child.y}`];

  if (child.x === routeX) segments.push(`V ${exitY}`);
  else segments.push(`C ${child.x} ${child.y + curve} ${routeX} ${exitY - curve} ${routeX} ${exitY}`);

  if (exitY !== entryY) segments.push(`V ${entryY}`);
  if (parent) {
    if (routeX === parent.x) segments.push(`V ${parent.y}`);
    else segments.push(`C ${routeX} ${entryY + curve} ${parent.x} ${parent.y - curve} ${parent.x} ${parent.y}`);
  }

  return segments.join(" ");
}

/**
 * Give every live parent edge a separate lane until its actual endpoint arrives.
 * Lanes may be reused only after that endpoint, never just because rows are adjacent.
 * In the normal --topo-order input, a node continues its first parent's lane. The
 * interval layout also preserves the actual endpoints if a caller supplies another order.
 */
export function buildHistoryGraph(commits: readonly GraphCommit[], rowHeight = historyRowHeight): HistoryGraph {
  const rowsHeight = commits.length * rowHeight;
  const rowByOid = new Map(commits.map((commit, row) => [commit.oid, row]));
  const openingEdges: PendingEdge[][] = commits.map(() => []);
  const pendingEdges: PendingEdge[] = [];

  for (const [childRow, commit] of commits.entries()) {
    for (const parentOid of new Set(commit.parents)) {
      const parentRow = rowByOid.get(parentOid) ?? null;
      const edge: PendingEdge = {
        id: `${commit.oid}:${parentOid}`,
        childOid: commit.oid,
        parentOid,
        childRow,
        parentRow,
        lowerRow: parentRow === null ? null : Math.max(childRow, parentRow),
        lane: -1,
      };
      const upperRow = parentRow === null ? childRow : Math.min(childRow, parentRow);
      openingEdges[upperRow].push(edge);
      pendingEdges.push(edge);
    }
  }

  const lanes: (PendingEdge | null)[] = [];
  const nodes: HistoryGraphNode[] = [];
  let laneCount = 0;

  for (const [row, commit] of commits.entries()) {
    const incomingLanes: number[] = [];
    lanes.forEach((edge, lane) => {
      if (edge?.lowerRow === row) incomingLanes.push(lane);
    });

    const nodeLane = incomingLanes[0] ?? firstFreeLane(lanes);
    nodes.push({
      oid: commit.oid,
      row,
      lane: nodeLane,
      x: laneX(nodeLane),
      y: row * rowHeight + rowHeight / 2,
      color: laneColor(nodeLane),
      isMerge: new Set(commit.parents).size > 1,
    });
    laneCount = Math.max(laneCount, nodeLane + 1);
    incomingLanes.forEach((lane) => { lanes[lane] = null; });

    openingEdges[row].forEach((edge, index) => {
      const lane = index === 0 ? nodeLane : firstFreeLane(lanes);
      edge.lane = lane;
      lanes[lane] = edge;
      laneCount = Math.max(laneCount, lane + 1);
    });
  }

  const edges = pendingEdges.map((edge): HistoryGraphEdge => ({
    id: edge.id,
    childOid: edge.childOid,
    parentOid: edge.parentOid,
    childRow: edge.childRow,
    parentRow: edge.parentRow,
    lane: edge.lane,
    color: laneColor(edge.lane),
    path: edgePath(edge, nodes, rowsHeight, rowHeight),
  }));
  const boundaries = edges.filter((edge) => edge.parentRow === null).map((edge): HistoryGraphBoundary => ({
    edgeId: edge.id,
    parentOid: edge.parentOid,
    lane: edge.lane,
    color: edge.color,
    x: laneX(edge.lane),
    yStart: rowsHeight,
    yEnd: rowsHeight + historyContinuationHeight - 10,
  }));

  return {
    nodes,
    edges,
    boundaries,
    laneCount,
    width: graphPadding * 2 + Math.max(0, laneCount - 1) * laneGap,
    height: rowsHeight + (boundaries.length ? historyContinuationHeight : 0),
    rowsHeight,
  };
}
