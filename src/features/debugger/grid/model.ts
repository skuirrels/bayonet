/**
 * Collection visualiser (DBG-6, Rider's data grid): a collection's elements as
 * rows and their members as columns. Built from DAP `variables` alone, so it
 * works with any adapter: a collection's children named `[0]`, `0`… are its
 * elements (netcoredbg, lldb-dap, debugpy, js-debug), and each element's own
 * children are its cells. Elements without members get one "Value" column.
 */
import type { Variable } from "@/bindings";

/** At most this many rows are read (one `variables` request each). */
export const MAX_ROWS = 500;

/** `[3]`, `3`, `[3,1]`: an element of a collection. */
export function isElementName(name: string): boolean {
  return /^\[?\d+(?:\s*,\s*\d+)*\]?$/.test(name.trim());
}

/** Adapter-made groupings and helpers, not members of the element. */
const SYNTHETIC = new Set([
  "Raw View",
  "Results View",
  "Static members",
  "Non-Public members",
  "special variables",
  "function variables",
  "class variables",
  "protected variables",
  "len()",
  "[[Prototype]]",
  "__proto__",
  "[raw]",
]);

export function isMemberColumn(v: Variable): boolean {
  return !SYNTHETIC.has(v.name) && !v.name.startsWith("[[");
}

export const VALUE_COLUMN = "Value";

export interface GridRow {
  /** The element's name, e.g. `[2]`. */
  index: string;
  element: Variable;
  /** Cell per column name; the element itself under {@link VALUE_COLUMN} when it has no members. */
  cells: Map<string, Variable>;
}

export interface Grid {
  columns: string[];
  rows: GridRow[];
  /** Elements beyond {@link MAX_ROWS} that were not read. */
  truncated: number;
}

/** The elements among a collection's children, in index order. */
export function elementsOf(children: readonly Variable[]): Variable[] {
  return children.filter((c) => isElementName(c.name));
}

/** `[0 … 99]`, `[100..199]`: a group of elements (js-debug, debugpy page large arrays). */
const RANGE = /^\[\s*(\d+)\s*(?:…|\.\.\.?)\s*(\d+)\s*\]$/;

/** The element count of a range group, or `null` when `name` is not one. */
export function rangeSize(name: string): number | null {
  const m = RANGE.exec(name.trim());
  return m ? Number(m[2]) - Number(m[1]) + 1 : null;
}

/** Children that hold a collection's elements when it has none of its own (LINQ results). */
const ELEMENT_HOLDERS = new Set(["Results View"]);

/** Fetches the children of a `variablesReference`. */
export type FetchChildren = (reference: number) => Promise<Variable[]>;

/** Elements read concurrently when fetching their members. */
const BATCH = 16;

/**
 * A .NET list shown as its fields (netcoredbg has no type proxies): `_items` holds
 * `_size` elements followed by spare capacity.
 */
export function backingArray(children: readonly Variable[]): { reference: number; count: number } | null {
  const items = children.find((c) => c.name === "_items" && c.variablesReference > 0);
  const size = children.find((c) => c.name === "_size") ?? children.find((c) => c.name === "Count");
  const count = size ? Number(size.value) : NaN;
  return items && Number.isInteger(count) && count >= 0 ? { reference: items.variablesReference, count } : null;
}

/**
 * Reads the collection at `reference` into a grid: its elements (descending into
 * range groups, result views and a list's backing array), at most {@link MAX_ROWS},
 * then each element's members. `cancelled` is polled between requests. When no
 * elements are found, `fallback` may name another reference to read them from (an
 * array copy of the collection).
 */
export async function loadGrid(
  fetch: FetchChildren,
  reference: number,
  cancelled: () => boolean = () => false,
  fallback?: () => Promise<number | null>,
): Promise<Grid> {
  const elements: Variable[] = [];
  let truncated = 0;
  const collect = async (ref: number, depth: number, limit = Infinity) => {
    const children = await fetch(ref);
    const own = elementsOf(children).slice(0, limit);
    for (const e of own) {
      if (elements.length < MAX_ROWS) elements.push(e);
      else truncated++;
    }
    if (own.length || depth > 2 || cancelled()) return;
    const backing = backingArray(children);
    if (backing) {
      if (backing.count > 0) await collect(backing.reference, depth + 1, backing.count);
      return;
    }
    for (const c of children) {
      const size = rangeSize(c.name);
      if (size == null && !ELEMENT_HOLDERS.has(c.name)) continue;
      if (c.variablesReference <= 0) continue;
      if (elements.length >= MAX_ROWS) {
        truncated += size ?? 0;
        continue;
      }
      if (cancelled()) return;
      await collect(c.variablesReference, depth + 1);
    }
  };
  await collect(reference, 0);
  if (!elements.length && fallback && !cancelled()) {
    const copy = await fallback().catch(() => null);
    if (copy != null && copy > 0) await collect(copy, 0);
  }
  const rows: { element: Variable; members: Variable[] | null }[] = elements.map((element) => ({
    element,
    members: null,
  }));
  for (let i = 0; i < rows.length; i += BATCH) {
    if (cancelled()) break;
    await Promise.all(
      rows.slice(i, i + BATCH).map(async (row) => {
        if (row.element.variablesReference <= 0) return;
        try {
          row.members = await fetch(row.element.variablesReference);
        } catch {
          row.members = null;
        }
      }),
    );
  }
  return buildGrid(rows, truncated);
}

/** Rows from each element and its members (`null`: it has none to show). */
export function buildGrid(
  elements: readonly { element: Variable; members: readonly Variable[] | null }[],
  truncated = 0,
): Grid {
  const columns: string[] = [];
  const seen = new Set<string>();
  const add = (c: string) => {
    if (!seen.has(c)) {
      seen.add(c);
      columns.push(c);
    }
  };
  const rows = elements.map(({ element, members }) => {
    const cells = new Map<string, Variable>();
    const shown = members?.filter(isMemberColumn) ?? [];
    if (!shown.length) {
      cells.set(VALUE_COLUMN, element);
      add(VALUE_COLUMN);
    }
    for (const m of shown) {
      if (cells.has(m.name)) continue;
      cells.set(m.name, m);
      add(m.name);
    }
    return { index: element.name, element, cells };
  });
  // "Value" (scalar elements) goes first when some elements are objects and some are not.
  const value = columns.indexOf(VALUE_COLUMN);
  if (value > 0) columns.unshift(...columns.splice(value, 1));
  return { columns, rows, truncated };
}

const NUMERIC = /^[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?[a-z]*$/i;

/** Rows sorted by `column` (numbers numerically, missing cells last); `index` sorts by position. */
export function sortRows(rows: readonly GridRow[], column: string | null, descending: boolean): GridRow[] {
  const position = new Map(rows.map((r, i) => [r, i]));
  if (column == null) return descending ? [...rows].reverse() : [...rows];
  const sorted = [...rows].sort((a, b) => {
    const x = a.cells.get(column)?.value;
    const y = b.cells.get(column)?.value;
    if (x == null || y == null)
      return x == null && y == null ? position.get(a)! - position.get(b)! : x == null ? 1 : -1;
    const order =
      NUMERIC.test(x) && NUMERIC.test(y)
        ? parseFloat(x) - parseFloat(y)
        : x.localeCompare(y, undefined, { numeric: true, sensitivity: "base" });
    const signed = descending ? -order : order;
    return signed || position.get(a)! - position.get(b)!;
  });
  return sorted;
}
