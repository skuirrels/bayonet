import { describe, expect, it } from "vitest";
import type { MethodSignature } from "@/bindings";
import { initialRows, moveRow, parseParameter, preview, rowsError, toChange, unchanged } from "./model";

const param = (text: string, name: string, modifiers: string[] = [], hasDefault = false) => ({
  text,
  name,
  modifiers,
  hasDefault,
});
const SIG: MethodSignature = {
  name: "Total",
  kind: "method",
  container: "Pricing",
  parameters: [param("decimal net", "net"), param("int count", "count"), param('string c = "GBP"', "c", [], true)],
  extension: false,
  nameRange: { start: { line: 1, col: 19 }, end: { line: 1, col: 24 } },
  header: 'public decimal Total<T>(decimal net, int count, string c = "GBP") where T : new()',
};

describe("change signature (CI-6)", () => {
  it("previews the moved, removed and added parameters", () => {
    let rows = initialRows(SIG);
    expect(unchanged(SIG, rows)).toBe(true);
    rows = moveRow(rows, 1, -1).filter((r) => r.from !== 2);
    rows.push({ key: 9, from: null, text: "bool rounded", value: "true" });
    expect(preview(SIG, rows)).toBe("public decimal Total<T>(int count, decimal net, bool rounded) where T : new()");
    expect(unchanged(SIG, rows)).toBe(false);
    expect(rowsError(SIG, rows)).toBeNull();
    expect(toChange(rows)).toEqual([
      { from: 1, text: "int count", value: null },
      { from: 0, text: "decimal net", value: null },
      { from: null, text: "bool rounded", value: "true" },
    ]);
  });

  it("reads parameter declarations", () => {
    expect(parseParameter("[NotNull] params Dictionary<string, int>[] maps")).toEqual({
      name: "maps",
      modifiers: ["params"],
      hasDefault: false,
    });
    expect(parseParameter('string s = "a = b"')).toMatchObject({ name: "s", hasDefault: true });
    expect(parseParameter("int").name).toBe("");
  });

  it("explains what cannot be applied", () => {
    const rows = initialRows(SIG);
    expect(rowsError(SIG, [{ ...rows[0]!, text: "decimal amount" }])).toContain("Rename");
    expect(rowsError(SIG, [...rows, { key: 5, from: null, text: "int extra", value: " " }])).toContain("value");
    expect(rowsError(SIG, [...rows, { key: 5, from: null, text: "int extra = 0", value: "" }])).toBeNull();
    expect(rowsError(SIG, [...rows, { key: 5, from: null, text: "int net", value: "1" }])).toContain("twice");
    expect(rowsError(SIG, [{ key: 5, from: null, text: "params int[] xs", value: "" }, ...rows])).toContain("last");
    expect(rowsError(SIG, [...rows, { key: 5, from: null, text: "int", value: "1" }])).toContain("name");
    const ext = { ...SIG, extension: true };
    expect(rowsError(ext, moveRow(rows, 0, 1))).toContain("this");
  });
});
