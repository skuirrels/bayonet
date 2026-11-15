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
 * A .NET `Dictionary<K, V>` or `HashSet<T>` shown as its fields: `_entries` holds
 * `_count` slots, removed ones (`next < -1`) on the free list.
 */
export function hashedEntries(
  children: readonly Variable[],
): { reference: number; count: number; live: number | null } | null {
  const entries = children.find((c) => c.name === "_entries");
  const number = (name: string) => Number(children.find((c) => c.name === name)?.value);
  const count = number("_count");
  if (!entries || !children.some((c) => c.name === "_buckets") || !Number.isInteger(count) || count < 0) return null;
  const free = number("_freeCount");
  return { reference: entries.variablesReference, count, live: Number.isInteger(free) ? count - free : null };
}

const field = (parts: readonly Variable[], ...names: string[]) => parts.find((p) => names.includes(p.name));

/**
 * The live entries among a hashed collection's slots: a dictionary entry as its
 * `Key` and `Value`, a set entry as its value. `undefined` when the layout is unknown.
 */
export function liveEntries(
  slots: readonly { slot: Variable; parts: readonly Variable[] }[],
): { element: Variable; members?: Variable[] }[] | undefined {
  const out: { element: Variable; members?: Variable[] }[] = [];
  for (const { slot, parts } of slots) {
    const next = Number(field(parts, "next", "Next")?.value);
    const value = field(parts, "value", "Value");
    if (!Number.isInteger(next) || !value) return undefined;
    if (next < -1) continue;
    const name = `[${out.length}]`;
    const key = field(parts, "key", "Key");
    if (key) {
      out.push({
        element: { ...slot, name, value: `[${key.value}, ${value.value}]` },
        members: [
          { ...key, name: "Key" },
          { ...value, name: "Value" },
        ],
      });
    } else {
      out.push({ element: { ...value, name } });
    }
  }
  return out;
}

/**
 * Reads the collection at `reference` into a grid: its elements (descending into
 * range groups, result views, a list's backing array and a dictionary's or set's entries), at most {@link MAX_ROWS},
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
  /** Members already known (dictionary entries), by element. */
  const known = new Map<Variable, Variable[]>();
  let truncated = 0;
  const collectEntries = async (ref: number, count: number, live: number | null) => {
    const slots = elementsOf(await fetch(ref)).slice(0, count);
    const read: { slot: Variable; parts: Variable[] }[] = [];
    for (let i = 0; i < slots.length && !cancelled(); i += BATCH) {
      // Free slots count towards `_count` too: read past MAX_ROWS only until it is full.
      if (elements.length + read.length >= MAX_ROWS) break;
      const batch = slots.slice(i, i + BATCH);
      read.push(
        ...(await Promise.all(
          batch.map(async (slot) => ({
            slot,
            parts: slot.variablesReference > 0 ? await fetch(slot.variablesReference) : [],
          })),
        )),
      );
    }
    const entries = liveEntries(read);
    if (!entries) return;
    let shown = 0;
    for (const { element, members } of entries) {
      if (elements.length >= MAX_ROWS) break;
      elements.push(element);
      if (members) known.set(element, members);
      shown++;
    }
    // `_count - _freeCount` live entries; without it, the unread slots are an upper bound.
    if (shown < count) truncated += live != null ? Math.max(0, live - shown) : slots.length - read.length;
  };
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
    const hashed = hashedEntries(children);
    if (hashed) {
      if (hashed.count > 0 && hashed.reference > 0) await collectEntries(hashed.reference, hashed.count, hashed.live);
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
  const rows: { element: Variable; members: Variable[] | null; known?: boolean }[] = elements.map((element) => {
    const members = known.get(element);
    return { element, members: members ?? null, known: members != null };
  });
  for (let i = 0; i < rows.length; i += BATCH) {
    if (cancelled()) break;
    await Promise.all(
      rows.slice(i, i + BATCH).map(async (row) => {
        if (row.known || row.element.variablesReference <= 0) return;
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
  let scalar = false;
  const rows = elements.map(({ element, members }) => {
    const cells = new Map<string, Variable>();
    const shown = members?.filter(isMemberColumn) ?? [];
    if (!shown.length) {
      cells.set(VALUE_COLUMN, element);
      add(VALUE_COLUMN);
      scalar = true;
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
  if (value > 0 && scalar) columns.unshift(...columns.splice(value, 1));
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
