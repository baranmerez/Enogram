import assert from "node:assert/strict";
import test from "node:test";
import { parseDiffLines } from "./diffLines.ts";

test("suppressed empty context lines preserve following hunk numbers and change kinds", () => {
  const patch = [
    "diff --git a/file.txt b/file.txt",
    "index 83db48f..6403dcc 100644",
    "--- a/file.txt",
    "+++ b/file.txt",
    "@@ -1,4 +1,4 @@",
    " first",
    "",
    "-old",
    "+new",
    " last",
    "",
  ].join("\n");

  const lines = parseDiffLines(patch);
  assert.equal(lines.length, 10);
  assert.deepEqual(lines.slice(5), [
    { kind: "context", text: "first", oldNumber: 1, newNumber: 1 },
    { kind: "context", text: "", oldNumber: 2, newNumber: 2 },
    { kind: "remove", text: "old", oldNumber: 3, newNumber: null },
    { kind: "add", text: "new", oldNumber: null, newNumber: 3 },
    { kind: "context", text: "last", oldNumber: 4, newNumber: 4 },
  ]);
});

test("empty lines remain metadata unless both hunk sides have remaining lines", () => {
  assert.deepEqual(parseDiffLines(""), []);
  for (const prefix of ["diff --git a/a b/a", "@@ -0,0 +1 @@", "@@ -1 +0,0 @@", "@@ -1 +1 @@\n-old\n+new"]) {
    assert.deepEqual(parseDiffLines(`${prefix}\n\n`).at(-1), {
      kind: "meta", text: "", oldNumber: null, newNumber: null,
    });
  }
});

test("standard context prefixes, source prefixes, and no-newline notices retain their meaning", () => {
  const lines = parseDiffLines("--- a/a\r\n+++ b/a\r\n@@ -5,2 +7,2 @@\r\n \r\n----literal\r\n\\ No newline at end of file\r\n++++literal\r\n\\ No newline at end of file\r\n");
  assert.deepEqual(lines.map(({ kind }) => kind), ["meta", "meta", "hunk", "context", "remove", "note", "add", "note"]);
  assert.deepEqual(lines[3], { kind: "context", text: "", oldNumber: 5, newNumber: 7 });
  assert.deepEqual(lines[4], { kind: "remove", text: "---literal", oldNumber: 6, newNumber: null });
  assert.deepEqual(lines[6], { kind: "add", text: "+++literal", oldNumber: null, newNumber: 8 });
});
