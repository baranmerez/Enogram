import assert from "node:assert/strict";
import test from "node:test";
import { parseDiffLines } from "./diffLines.ts";
import { diffSearchMatchLimit, findDiffHunks, findDiffMatches, stepDiffIndex } from "./diffSearch.ts";

test("literal case-insensitive search never interprets metacharacters and keeps original offsets", () => {
  const lines = [{ text: "İ 😀 [A].*+?^${}()|\\ [a].*+?^${}()|\\" }];
  const query = "[a].*+?^${}()|\\";
  const { matches, limited } = findDiffMatches(lines, query);
  assert.equal(limited, false);
  assert.deepEqual(matches, [
    { lineIndex: 0, start: 5, end: 20 },
    { lineIndex: 0, start: 21, end: 36 },
  ]);
  assert.deepEqual(matches.map(({ start, end }) => lines[0].text.slice(start, end)), ["[A].*+?^${}()|\\", query]);
  assert.deepEqual(findDiffMatches([{ text: "İ A a" }], "a").matches, [
    { lineIndex: 0, start: 2, end: 3 }, { lineIndex: 0, start: 4, end: 5 },
  ]);
  assert.equal(findDiffMatches([{ text: "Σ σ ς" }], "σ").matches.length, 3);
});

test("spaces and tabs are literal, matches do not overlap or span patch lines", () => {
  assert.deepEqual(findDiffMatches([{ text: "A\t  a\t " }], "\t ").matches, [
    { lineIndex: 0, start: 1, end: 3 }, { lineIndex: 0, start: 5, end: 7 },
  ]);
  assert.deepEqual(findDiffMatches([{ text: "AaAa" }], "aa").matches, [
    { lineIndex: 0, start: 0, end: 2 }, { lineIndex: 0, start: 2, end: 4 },
  ]);
  assert.deepEqual(findDiffMatches([{ text: "a" }, { text: "b" }], "a\nb").matches, []);
  assert.deepEqual(findDiffMatches([{ text: "a" }], ""), { matches: [], limited: false });
});

test("search uses only displayed text while hunk navigation uses parsed hunk rows", () => {
  const lines = parseDiffLines("--- a/file.txt\n+++ b/file.txt\n@@ -10,2 +20,2 @@\n-old\n+new\n same\n@@ -80 +90 @@\n-OLD\n+NEW\n");
  assert.deepEqual(findDiffHunks(lines), [2, 6]);
  assert.deepEqual(findDiffMatches(lines, "old").matches, [
    { lineIndex: 3, start: 0, end: 3 }, { lineIndex: 7, start: 0, end: 3 },
  ]);
  assert.equal(findDiffMatches(lines, "file.txt").matches.length, 2, "Displayed metadata is searchable");
  assert.deepEqual(findDiffMatches(lines, "-old").matches, [], "Source change markers are not part of source text");
  assert.deepEqual(findDiffMatches(lines, "20").matches, [{ lineIndex: 2, start: 10, end: 12 }]);
  assert.equal(lines.length, 9, "Final line ending must not add a searchable row");
});

test("large generated diffs retain bounded highlights and report only real overflow", () => {
  const { matches, limited } = findDiffMatches([{ text: "a".repeat(4 * 1024 * 1024) }], "a");
  assert.equal(limited, true);
  assert.equal(matches.length, diffSearchMatchLimit);
  assert.equal(matches.at(-1).end, diffSearchMatchLimit);
  assert.equal(findDiffMatches([{ text: "a".repeat(diffSearchMatchLimit) }], "a").limited, false);
  assert.equal(findDiffMatches([{ text: "a".repeat(diffSearchMatchLimit) }, { text: "A" }], "a").limited, true);
});

test("navigation wraps in either direction and handles an empty or reset selection", () => {
  assert.equal(stepDiffIndex(0, 0, 1), -1);
  assert.equal(stepDiffIndex(-1, 3, 1), 0);
  assert.equal(stepDiffIndex(-1, 3, -1), 2);
  assert.equal(stepDiffIndex(2, 3, 1), 0);
  assert.equal(stepDiffIndex(0, 3, -1), 2);
  assert.equal(stepDiffIndex(4, 2, 1), 0);
  assert.equal(stepDiffIndex(0, 1, -1), 0);
});
