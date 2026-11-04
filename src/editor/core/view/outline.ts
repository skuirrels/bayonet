import type { OutlineItem, Pos } from "@/platform/ipc";

const before = (a: Pos, b: Pos) => a.line < b.line || (a.line === b.line && a.col <= b.col);

/** Outline items enclosing (line, col), outermost first. */
export function symbolChain(items: readonly OutlineItem[], line: number, col: number): OutlineItem[] {
  const pos = { line, col };
  const out: OutlineItem[] = [];
  let level: readonly OutlineItem[] = items;
  for (;;) {
    const hit = level.find((i) => before(i.range.start, pos) && before(pos, i.range.end));
    if (!hit) return out;
    out.push(hit);
    level = hit.children;
  }
}
