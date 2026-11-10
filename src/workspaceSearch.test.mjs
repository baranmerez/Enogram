import assert from "node:assert/strict";
import test from "node:test";
import { filterFiles, findCommits } from "./workspaceSearch.ts";
import { stagePaths, unstagePaths } from "./workingChanges.ts";

const files = [
  { path: "src/Main.ts", originalPath: null, indexStatus: "M", worktreeStatus: "M" },
  { path: "docs/new.md", originalPath: "docs/old.md", indexStatus: "R", worktreeStatus: " " },
  { path: "new.txt", originalPath: null, indexStatus: " ", worktreeStatus: "?" },
  { path: "conflict.txt", originalPath: null, indexStatus: "U", worktreeStatus: "U" },
];
test("file filters match paths and rename origins, retaining both partial-stage layers", () => {
  assert.deepEqual(filterFiles(files, " SRC\\main "), [files[0]]);
  assert.deepEqual(filterFiles(files, "old.md", "staged"), [files[1]]);
  assert.deepEqual(filterFiles(files, "", "working"), [files[0], files[2], files[3]]);
  assert.deepEqual(filterFiles(files, "", "staged"), [files[0], files[1], files[3]]);
  assert.deepEqual(filterFiles(files, "", "conflicts"), [files[3]]);
  assert.deepEqual(stagePaths(filterFiles(files, "", "conflicts")), []);
  assert.deepEqual(unstagePaths(filterFiles(files, "Main")), [files[0].path]);
  assert.deepEqual(filterFiles(files, "absent"), []);
});
test("loaded history search finds subject, author, SHA and refs without rewriting graph data", () => {
  const commits = [{ oid: "abc123", subject: "Fix search", author: "Example", refs: "HEAD -> main", parents: ["older"], date: "" }];
  for (const query of [" FIX ", "EXAMPLE", "abc", "MAIN"]) assert.deepEqual(findCommits(commits, query), commits);
  assert.deepEqual(findCommits(commits, "not found"), []);
  assert.deepEqual(findCommits(commits, "  "), []);
  assert.deepEqual(commits[0].parents, ["older"]);
});
