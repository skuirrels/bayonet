import { describe, expect, it } from "vitest";
import { buildFileTree, dirKeys } from "../fileTree";

describe("buildFileTree", () => {
  it("groups by directory, merges single-child chains and counts files", () => {
    const tree = buildFileTree(["src/app/a.ts", "src/app/b.ts", "README.md", "docs/x/y/z.md"], (p) => p);
    expect(tree.map((n) => n.name)).toEqual(["docs/x/y", "src/app", "README.md"]);
    expect(tree[1]!.count).toBe(2);
    expect(tree[1]!.children.map((c) => c.name)).toEqual(["a.ts", "b.ts"]);
    expect(dirKeys(tree)).toEqual(["d:docs/x/y", "d:src/app"]);
  });
});
