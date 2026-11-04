/**
 * Multi-selection of the Find in Files results list: click / ↑↓ select one row,
 * ⇧ extends from the anchor, ⌘/Ctrl toggles a row (IDEA's results table).
 */

export interface RowSelection {
  /** Active row (keyboard focus, preview). */
  lead: number;
  /** Start of ⇧ ranges. */
  anchor: number;
  /** Keys of the selected rows; `null` when only the lead is selected. */
  keys: ReadonlySet<string> | null;
}

export const SINGLE_FIRST: RowSelection = { lead: 0, anchor: 0, keys: null };

export function selectRow(
  rows: ReadonlyArray<{ key: string }>,
  sel: RowSelection,
  index: number,
  mode: "single" | "range" | "toggle" = "single",
): RowSelection {
  if (index < 0 || index >= rows.length) return sel;
  if (mode === "range") {
    const anchor = Math.min(Math.max(0, sel.anchor), rows.length - 1);
    const [a, b] = anchor <= index ? [anchor, index] : [index, anchor];
    return { lead: index, anchor, keys: new Set(rows.slice(a, b + 1).map((r) => r.key)) };
  }
  if (mode === "toggle") {
    const keys = new Set(sel.keys ?? (rows[sel.lead] ? [rows[sel.lead]!.key] : []));
    const key = rows[index]!.key;
    if (!keys.has(key)) return { lead: index, anchor: index, keys: keys.add(key) };
    keys.delete(key);
    const rest = rows.findLastIndex((r) => keys.has(r.key));
    if (rest < 0) return { lead: index, anchor: index, keys: null };
    return { lead: rest, anchor: rest, keys };
  }
  return { lead: index, anchor: index, keys: null };
}

/** The selected rows in list order (the lead when nothing else is selected). */
export function selectedRows<T extends { key: string }>(rows: readonly T[], sel: RowSelection): T[] {
  if (!rows.length) return [];
  const lead = Math.min(sel.lead, rows.length - 1);
  if (!sel.keys) return [rows[lead]!];
  const out = rows.filter((r) => sel.keys!.has(r.key));
  return out.length ? out : [rows[lead]!];
}

export function isRowSelected(sel: RowSelection, key: string): boolean {
  return !!sel.keys?.has(key);
}
