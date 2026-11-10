import assert from "node:assert/strict";
import test from "node:test";
import { hasStatus, isConflicted, stagePaths, unstagePaths } from "./workingChanges.ts";

const file = (path, indexStatus = ".", worktreeStatus = ".", originalPath = null) => ({
  path, indexStatus, worktreeStatus, originalPath,
});

test("empty porcelain statuses do not become mutation targets", () => {
  for (const status of ["", " ", ".", "-"]) assert.equal(hasStatus(status), false);
  assert.deepEqual(stagePaths([file("clean.txt")]), []);
  assert.deepEqual(unstagePaths([file("clean.txt")]), []);
});

test("partial staging keeps separate worktree and index path sets", () => {
  const changes = [
    file("both.txt", "M", "M"),
    file("index.txt", "A"),
    file("worktree.txt", ".", "M"),
    file("new.txt", ".", "?"),
    file("deleted.txt", ".", "D"),
  ];
  assert.deepEqual(stagePaths(changes), ["both.txt", "worktree.txt", "new.txt", "deleted.txt"]);
  assert.deepEqual(unstagePaths(changes), ["both.txt", "index.txt"]);
});

test("all unmerged porcelain pairs block their own paths, not unrelated changes", () => {
  const conflicts = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"]
    .map((pair) => file(`${pair}.txt`, pair[0], pair[1]));
  for (const conflict of conflicts) assert.equal(isConflicted(conflict), true);
  assert.equal(isConflicted(file("partial.txt", "M", "M")), false);
  const changes = [...conflicts, file("ready.txt", "M", "M")];
  assert.deepEqual(stagePaths(changes), ["ready.txt"]);
  assert.deepEqual(unstagePaths(changes), ["ready.txt"]);
});

test("explicit status paths preserve rename destinations and literal filenames", () => {
  const changes = [
    file("renamed file.txt", "R", "M", "old file.txt"),
    file("--literal.txt", ".", "?"),
    file("çalışma/şema.ts", ".", "M"),
    file("renamed file.txt", "R", "M", "old file.txt"),
  ];
  assert.deepEqual(stagePaths(changes), ["renamed file.txt", "--literal.txt", "çalışma/şema.ts"]);
  assert.deepEqual(unstagePaths(changes), ["renamed file.txt"]);
});
