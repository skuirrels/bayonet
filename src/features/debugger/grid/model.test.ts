import { describe, expect, it } from "vitest";
import type { Variable } from "@/bindings";
import {
  MAX_ROWS,
  backingArray,
  buildGrid,
  elementsOf,
  isElementName,
  loadGrid,
  rangeSize,
  sortRows,
  type FetchChildren,
} from "./model";

const v = (name: string, value: string, variablesReference = 0): Variable => ({
  name,
  value,
  typeName: null,
  variablesReference,
  evaluateName: null,
});

/** A fake adapter: children per reference. */
const adapter = (tree: Record<number, Variable[]>): FetchChildren & { calls: number[] } => {
  const calls: number[] = [];
  const fetch = (ref: number) => {
    calls.push(ref);
    return Promise.resolve(tree[ref] ?? []);
  };
  return Object.assign(fetch, { calls });
};

describe("collection visualiser (DBG-6)", () => {
  it("recognises element names and range groups", () => {
    expect(["[0]", "12", "[1, 2]", " [3] "].every(isElementName)).toBe(true);
    expect(["Count", "[0 … 99]", "Raw View", "x1"].some(isElementName)).toBe(false);
    expect(rangeSize("[0 … 99]")).toBe(100);
    expect(rangeSize("[100..199]")).toBe(100);
    expect(rangeSize("[0]")).toBeNull();
  });

  it("keeps the elements, not the collection's own members", () => {
    const children = [v("Count", "2"), v("[0]", "a"), v("[1]", "b"), v("Raw View", "")];
    expect(elementsOf(children).map((c) => c.name)).toEqual(["[0]", "[1]"]);
  });

  it("makes columns from the members and puts scalar elements under Value first", () => {
    const grid = buildGrid([
      { element: v("[0]", "Point"), members: [v("X", "1"), v("Y", "2"), v("Raw View", "")] },
      { element: v("[1]", "null"), members: null },
      { element: v("[2]", "Point"), members: [v("X", "3"), v("Z", "9")] },
    ]);
    expect(grid.columns).toEqual(["Value", "X", "Y", "Z"]);
    expect(grid.rows[1]?.cells.get("Value")?.value).toBe("null");
    expect(grid.rows[2]?.cells.has("Y")).toBe(false);
  });

  it("sorts numbers numerically with missing cells last, and by index in reverse", () => {
    const grid = buildGrid([
      { element: v("[0]", "o"), members: [v("n", "10")] },
      { element: v("[1]", "o"), members: [v("m", "1")] },
      { element: v("[2]", "o"), members: [v("n", "9")] },
    ]);
    const order = (column: string | null, descending: boolean) =>
      sortRows(grid.rows, column, descending).map((r) => r.index);
    expect(order("n", false)).toEqual(["[2]", "[0]", "[1]"]);
    expect(order("n", true)).toEqual(["[0]", "[2]", "[1]"]);
    expect(order(null, true)).toEqual(["[2]", "[1]", "[0]"]);
  });

  it("loads elements through range groups and reads each element's members", async () => {
    const fetch = adapter({
      1: [v("length", "4"), v("[0 … 1]", "", 2), v("[2 … 3]", "", 3)],
      2: [v("0", "{…}", 10), v("1", "{…}", 11)],
      3: [v("2", "7"), v("3", "{…}", 13)],
      10: [v("name", "'a'"), v("[[Prototype]]", "Object", 99)],
      11: [v("name", "'b'")],
      13: [v("name", "'d'")],
    });
    const grid = await loadGrid(fetch, 1);
    expect(grid.rows.map((r) => r.index)).toEqual(["0", "1", "2", "3"]);
    expect(grid.columns).toEqual(["Value", "name"]);
    expect(grid.rows[2]?.cells.get("Value")?.value).toBe("7");
    expect(fetch.calls).not.toContain(99);
  });

  it("reads at most MAX_ROWS elements and counts the rest", async () => {
    const many = Array.from({ length: MAX_ROWS + 20 }, (_, i) => v(`[${i}]`, String(i)));
    const fetch = adapter({ 1: many });
    const grid = await loadGrid(fetch, 1);
    expect(grid.rows).toHaveLength(MAX_ROWS);
    expect(grid.truncated).toBe(20);
  });

  it("stops reading members once cancelled", async () => {
    const elements = Array.from({ length: 40 }, (_, i) => v(`[${i}]`, "o", 100 + i));
    const fetch = adapter({ 1: elements });
    let cancelled = false;
    const loading = loadGrid(fetch, 1, () => cancelled);
    cancelled = true;
    await loading;
    expect(fetch.calls.filter((r) => r >= 100).length).toBeLessThan(40);
  });

  // What netcoredbg shows for a List<T> (no type proxies), from the real adapter test.
  it("reads a .NET list's backing array up to its size", async () => {
    const fetch = adapter({
      1: [
        v("_items", "{Order[4]}", 2),
        v("_size", "3"),
        v("Capacity", "4"),
        v("Count", "3"),
        v("Static members", "", 9),
      ],
      2: [v("[0]", "1"), v("[1]", "2"), v("[2]", "3"), v("[3]", "null")],
    });
    expect(backingArray([v("_items", "", 2), v("Count", "2")])).toEqual({ reference: 2, count: 2 });
    const grid = await loadGrid(fetch, 1);
    expect(grid.rows.map((r) => r.index)).toEqual(["[0]", "[1]", "[2]"]);
    expect(fetch.calls).not.toContain(9);
  });

  it("reads an array copy when the collection shows no elements", async () => {
    const fetch = adapter({ 1: [v("_buckets", "{int[3]}", 3)], 5: [v("[0]", "a"), v("[1]", "b")] });
    const grid = await loadGrid(fetch, 1, undefined, () => Promise.resolve(5));
    expect(grid.rows.map((r) => r.index)).toEqual(["[0]", "[1]"]);
    const failing = await loadGrid(fetch, 1, undefined, () => Promise.reject(new Error("no ToArray")));
    expect(failing.rows).toEqual([]);
  });
});
