import type { DiffLine } from "./diffLines";

export interface DiffSearchMatch {
  lineIndex: number;
  start: number;
  end: number;
}

export const diffSearchMatchLimit = 2_000;

export interface DiffSearchResult {
  matches: DiffSearchMatch[];
  limited: boolean;
}

export function findDiffMatches(lines: readonly Pick<DiffLine, "text">[], query: string): DiffSearchResult {
  if (!query) return { matches: [], limited: false };

  // Escape search text instead of interpreting patterns. Match against the
  // original text so Unicode case folding cannot shift highlight offsets.
  const pattern = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu");
  const matches: DiffSearchMatch[] = [];
  for (const [lineIndex, line] of lines.entries()) {
    pattern.lastIndex = 0;
    for (const match of line.text.matchAll(pattern)) {
      // A short query in a generated file can otherwise create millions of DOM
      // marks. Stop at the first omitted match and expose that boundary to the UI.
      if (matches.length === diffSearchMatchLimit) return { matches, limited: true };
      matches.push({ lineIndex, start: match.index, end: match.index + match[0].length });
    }
  }
  return { matches, limited: false };
}

export function findDiffHunks(lines: readonly Pick<DiffLine, "kind">[]): number[] {
  return lines.flatMap((line, index) => line.kind === "hunk" ? [index] : []);
}

export function stepDiffIndex(index: number, count: number, direction: 1 | -1): number {
  if (count === 0) return -1;
  if (index < 0 || index >= count) return direction === 1 ? 0 : count - 1;
  return (index + direction + count) % count;
}
