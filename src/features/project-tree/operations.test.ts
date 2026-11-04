import { describe, expect, it } from "vitest";
import type { ProjectNode } from "./model";
import { selectionAfterDelete } from "./operations";

const node = (path: string, dir = false): ProjectNode => ({
  key: path,
  path,
  chain: [path],
  label: path.slice(path.lastIndexOf("/") + 1),
  dir,
  root: path === "/p",
  entry: null,
  ignored: false,
  excluded: false,
  symlink: false,
});

describe("selectionAfterDelete", () => {
  const rows = [node("/p", true), node("/p/a", true), node("/p/a/x.ts"), node("/p/b", true), node("/p/c.md")];

  it("selects the next surviving row, skipping the deleted folder's children", () => {
    expect(selectionAfterDelete(rows, ["/p/a"])).toBe("/p/b");
  });

  it("falls back to the previous row at the end of the list", () => {
    expect(selectionAfterDelete(rows, ["/p/c.md"])).toBe("/p/b");
    expect(selectionAfterDelete(rows, ["/p/b", "/p/c.md"])).toBe("/p/a/x.ts");
  });

  it("returns null when nothing visible was deleted", () => {
    expect(selectionAfterDelete(rows, ["/elsewhere"])).toBeNull();
  });
});
