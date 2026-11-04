import type { TextChange } from "@/platform/ipc";

/**
 * The single replacement turning `a` into `b` (common prefix / suffix
 * trimmed), or null when equal. Linear time; used for resyncs, where the
 * backend's own patches are already minimal.
 */
export function minimalChange(a: string, b: string): TextChange | null {
  if (a === b) return null;
  const max = Math.min(a.length, b.length);
  let start = 0;
  while (start < max && a.charCodeAt(start) === b.charCodeAt(start)) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a.charCodeAt(endA - 1) === b.charCodeAt(endB - 1)) {
    endA--;
    endB--;
  }
  return { from: start, to: endA, insert: b.slice(start, endB) };
}
