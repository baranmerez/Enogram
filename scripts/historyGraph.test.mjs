import assert from "node:assert/strict";
import test from "node:test";
import { buildHistoryGraph, compactHistoryRowHeight, historyContinuationHeight, historyRowHeight, parseCommitRefs } from "../src/historyGraph.ts";

const commit = (oid, ...parents) => ({ oid, parents });
const edgeIds = (graph) => graph.edges.map((edge) => `${edge.childOid}>${edge.parentOid}`).sort();

function assertActualEdges(commits, graph) {
  assert.deepEqual(edgeIds(graph), commits.flatMap(({ oid, parents }) => [...new Set(parents)].map((parent) => `${oid}>${parent}`)).sort());
  for (const edge of graph.edges) {
    const child = graph.nodes[edge.childRow];
    assert.equal(child.oid, edge.childOid);
    assert.ok(edge.path.startsWith(`M ${child.x} ${child.y} `), `Edge ${edge.id} must start at its child`);
    if (edge.parentRow !== null) {
      const parent = graph.nodes[edge.parentRow];
      assert.equal(parent.oid, edge.parentOid);
      assert.ok(edge.path.endsWith(` ${parent.y}`), `Edge ${edge.id} must end at its parent row`);
      assert.equal(graph.boundaries.some((boundary) => boundary.edgeId === edge.id), false);
    } else {
      const boundary = graph.boundaries.find((item) => item.edgeId === edge.id);
      assert.ok(boundary, `Only a missing parent needs a continuation: ${edge.id}`);
      assert.equal(boundary.parentOid, edge.parentOid);
      assert.ok(edge.path.endsWith(` ${graph.rowsHeight}`));
    }
  }

  // No routed edge may pass through an unrelated node or occupy another live edge's lane.
  for (const node of graph.nodes) {
    const passing = graph.edges.filter((edge) => {
      const first = Math.min(edge.childRow, edge.parentRow ?? Infinity);
      const last = Math.max(edge.childRow, edge.parentRow ?? Infinity);
      return first < node.row && node.row < last;
    });
    assert.ok(passing.every((edge) => edge.lane !== node.lane), `No unrelated edge through ${node.oid}`);
    assert.equal(new Set(passing.map((edge) => edge.lane)).size, passing.length, `Live lanes are distinct at ${node.oid}`);
  }
}

test("linear history uses a continuous lane and only actual parent links", () => {
  const commits = [commit("c", "b"), commit("b", "a"), commit("a")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["b>a", "c>b"]);
  assert.deepEqual(graph.nodes.map((node) => node.lane), [0, 0, 0]);
  assert.equal(new Set(graph.edges.map((edge) => edge.color)).size, 1);
  assert.equal(graph.boundaries.length, 0);
  assert.equal(graph.height, commits.length * historyRowHeight);
});

test("a real merge diamond forks at the merge and converges at the shared ancestor", () => {
  const commits = [commit("merge", "main", "topic"), commit("main", "base"), commit("topic", "base"), commit("base")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["main>base", "merge>main", "merge>topic", "topic>base"]);
  assert.deepEqual(graph.nodes.map((node) => node.lane), [0, 0, 1, 0]);
  assert.deepEqual(graph.edges.map((edge) => edge.lane), [0, 1, 0, 1]);
  assert.equal(graph.nodes[0].isMerge, true);
  assert.equal(graph.edges[0].color, graph.edges[2].color);
  assert.equal(graph.edges[1].color, graph.edges[3].color);
  assert.notEqual(graph.edges[0].color, graph.edges[1].color);
  assert.equal(graph.edges[3].parentRow, 3);
  assert.equal(graph.laneCount, 2);
});

test("diverging tips stay separate until their actual common parent", () => {
  const commits = [commit("left", "base"), commit("right", "base"), commit("base")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["left>base", "right>base"]);
  assert.deepEqual(graph.nodes.map((node) => node.lane), [0, 1, 0]);
  assert.deepEqual(graph.edges.map((edge) => edge.lane), [0, 1]);
  assert.ok(graph.edges.every((edge) => edge.parentRow === 2));
});

test("octopus merges retain every parent and remain wider than two lanes", () => {
  const commits = [
    commit("octopus", "a", "b", "c", "d"),
    commit("a", "base"), commit("b", "base"), commit("c", "base"), commit("d", "base"), commit("base"),
  ];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["a>base", "b>base", "c>base", "d>base", "octopus>a", "octopus>b", "octopus>c", "octopus>d"]);
  assert.equal(graph.laneCount, 4);
  assert.deepEqual(graph.nodes.map((node) => node.lane), [0, 0, 1, 2, 3, 0]);
});

test("only actual missing parents continue beyond the loaded window", () => {
  const commits = [commit("tip", "inside", "outsideB"), commit("inside", "outsideA"), commit("unrelatedRoot")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["inside>outsideA", "tip>inside", "tip>outsideB"]);
  assert.deepEqual(graph.boundaries.map((boundary) => boundary.parentOid).sort(), ["outsideA", "outsideB"]);
  assert.equal(graph.height, graph.rowsHeight + historyContinuationHeight);
  assert.equal(graph.nodes[2].lane, 2);
  assert.ok(graph.boundaries.every((boundary) => boundary.yStart === graph.rowsHeight && boundary.yEnd > graph.rowsHeight));
});

test("disconnected roots and a full 80-row page receive no invented connectors", () => {
  const commits = Array.from({ length: 80 }, (_, index) => commit(`root${index}`));
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(graph.edges, []);
  assert.deepEqual(graph.boundaries, []);
  assert.ok(graph.nodes.every((node) => node.lane === 0));
  assert.equal(graph.height, 80 * historyRowHeight);
});

test("interleaved disconnected branches reuse lanes only after their roots", () => {
  const commits = [commit("a2", "a1"), commit("b2", "b1"), commit("a1"), commit("c2", "c1"), commit("b1"), commit("c1")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["a2>a1", "b2>b1", "c2>c1"]);
  assert.deepEqual(graph.nodes.map((node) => node.lane), [0, 1, 0, 0, 1, 0]);
  assert.equal(graph.laneCount, 2);
});

test("layout and colors are deterministic and do not mutate input", () => {
  const commits = Object.freeze([
    Object.freeze({ oid: "merge", parents: Object.freeze(["one", "two"]) }),
    Object.freeze({ oid: "one", parents: Object.freeze(["older"]) }),
    Object.freeze({ oid: "two", parents: Object.freeze(["older"]) }),
  ]);
  assert.deepEqual(buildHistoryGraph(commits), buildHistoryGraph(commits));
});

test("a visible parent in another row order is still an actual endpoint, not a boundary", () => {
  const commits = [commit("root"), commit("middle", "root"), commit("tip", "middle")];
  const graph = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.deepEqual(edgeIds(graph), ["middle>root", "tip>middle"]);
  assert.equal(graph.boundaries.length, 0);
  assert.ok(graph.edges.every((edge) => edge.parentRow < edge.childRow));
});

test("compact merge curves align with 36px row centers and keep the same parent lanes", () => {
  const commits = [commit("merge", "main", "topic"), commit("main", "base"), commit("topic", "base"), commit("base")];
  const graph = buildHistoryGraph(commits, compactHistoryRowHeight);
  const comfortable = buildHistoryGraph(commits);
  assertActualEdges(commits, graph);
  assert.equal(compactHistoryRowHeight, 36);
  assert.deepEqual(graph.nodes.map((node) => node.y), [18, 54, 90, 126]);
  assert.deepEqual(graph.nodes.map((node) => node.lane), comfortable.nodes.map((node) => node.lane));
  assert.deepEqual(graph.edges.map((edge) => edge.lane), comfortable.edges.map((edge) => edge.lane));
  assert.equal(graph.width, comfortable.width);
  assert.equal(graph.height, 144);
  assert.equal(graph.edges.find((edge) => edge.id === "merge:topic").path, "M 16 18 C 16 27 36 27 36 36 V 72 V 90");
  assert.equal(graph.edges.find((edge) => edge.id === "topic:base").path, "M 36 90 V 108 C 36 117 16 117 16 126");
});

test("compact missing-parent continuations start after the actual final row", () => {
  const commits = [commit("tip", "inside", "outsideB"), commit("inside", "outsideA"), commit("unrelatedRoot")];
  const graph = buildHistoryGraph(commits, compactHistoryRowHeight);
  assertActualEdges(commits, graph);
  assert.equal(graph.rowsHeight, 108);
  assert.equal(graph.height, 108 + historyContinuationHeight);
  assert.ok(graph.boundaries.every((boundary) => boundary.yStart === 108 && boundary.yEnd === 132));
  assert.equal(graph.nodes[2].y, 90);
});

test("custom row height also routes upward parents and preserves default geometry", () => {
  const commits = [commit("root"), commit("middle", "root"), commit("tip", "middle")];
  const graph = buildHistoryGraph(commits, compactHistoryRowHeight);
  assertActualEdges(commits, graph);
  assert.equal(graph.edges[0].path, "M 16 54 V 36 V 18");
  assert.equal(graph.edges[1].path, "M 16 90 V 72 V 54");
  assert.deepEqual(buildHistoryGraph(commits), buildHistoryGraph(commits, historyRowHeight));
  assert.equal(buildHistoryGraph(commits, 64).nodes[2].y, 160);
  assert.equal(buildHistoryGraph([], compactHistoryRowHeight).height, 0);
});

test("empty history produces no graph or continuation", () => {
  const graph = buildHistoryGraph([]);
  assert.deepEqual(graph.nodes, []);
  assert.deepEqual(graph.edges, []);
  assert.deepEqual(graph.boundaries, []);
  assert.equal(graph.height, 0);
  assert.equal(graph.laneCount, 0);
});

test("Git decorations preserve HEAD, tags, comma-containing names and slash branches", () => {
  assert.deepEqual(parseCommitRefs("HEAD -> main, tag: v1.2, origin/main, feature/one,two"), [
    { label: "HEAD → main", kind: "head" },
    { label: "tag: v1.2", kind: "tag" },
    { label: "origin/main", kind: "branch" },
    { label: "feature/one,two", kind: "branch" },
  ]);
  assert.deepEqual(parseCommitRefs("HEAD"), [{ label: "HEAD", kind: "head" }]);
  assert.deepEqual(parseCommitRefs(""), []);
});
