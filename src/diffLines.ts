export type DiffLineKind = "add" | "remove" | "context" | "hunk" | "meta" | "note";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  oldNumber: number | null;
  newNumber: number | null;
}

export function parseDiffLines(text: string): DiffLine[] {
  if (!text) return [];

  const lines = text.replace(/\r\n/g, "\n").split("\n");
  // The final line ending terminates a patch line; it is not another source line.
  if (lines.at(-1) === "") lines.pop();

  let oldNumber = 0;
  let newNumber = 0;
  let oldRemaining = 0;
  let newRemaining = 0;

  return lines.map((line): DiffLine => {
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (hunk) {
      oldNumber = Number(hunk[1]);
      newNumber = Number(hunk[3]);
      oldRemaining = hunk[2] === undefined ? 1 : Number(hunk[2]);
      newRemaining = hunk[4] === undefined ? 1 : Number(hunk[4]);
      return { kind: "hunk", text: line, oldNumber: null, newNumber: null };
    }

    if (line.startsWith("\\ ")) {
      return { kind: "note", text: line, oldNumber: null, newNumber: null };
    }

    // Interpret prefixes only inside a hunk, so file headers stay metadata and
    // source lines beginning with "+++" or "---" retain their literal content.
    if (line.startsWith("+") && newRemaining > 0) {
      newRemaining -= 1;
      return { kind: "add", text: line.slice(1), oldNumber: null, newNumber: newNumber++ };
    }
    if (line.startsWith("-") && oldRemaining > 0) {
      oldRemaining -= 1;
      return { kind: "remove", text: line.slice(1), oldNumber: oldNumber++, newNumber: null };
    }
    // Git's diff.suppressBlankEmpty can omit the context prefix on empty lines.
    if ((line.startsWith(" ") || line === "") && oldRemaining > 0 && newRemaining > 0) {
      oldRemaining -= 1;
      newRemaining -= 1;
      return { kind: "context", text: line.slice(1), oldNumber: oldNumber++, newNumber: newNumber++ };
    }

    oldRemaining = 0;
    newRemaining = 0;
    return { kind: "meta", text: line, oldNumber: null, newNumber: null };
  });
}
