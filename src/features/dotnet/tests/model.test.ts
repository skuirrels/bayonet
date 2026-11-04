import { describe, expect, it } from "vitest";
import type { TestNode } from "@/bindings";
import { buildTree, countTests, mergeTest, nodeStatus, selectionOf, stackLocations, type ProjectTests } from "./model";
import { gutterTargets } from "./gutter";

const P = "/s/tests/T.Tests/T.Tests.csproj";

function test(uid: string, over: Partial<TestNode> = {}): TestNode {
  return {
    uid,
    displayName: uid,
    typeName: "N.GreeterTests",
    method: uid,
    file: "/s/tests/T.Tests/UnitTest1.cs",
    line: 7,
    state: "discovered",
    durationMs: null,
    message: null,
    stackTrace: null,
    expected: null,
    actual: null,
    output: null,
    ...over,
  };
}

const project = (tests: TestNode[]): ProjectTests => ({
  path: P,
  name: "T.Tests",
  tests: new Map(tests.map((t) => [t.uid, t])),
  message: null,
  busy: false,
});

const xunit = [
  test("a", {
    displayName: "N.GreeterTests.Greets_by_name",
    method: "Greets_by_name",
    state: "passed",
  }),
  test("b", {
    displayName: 'N.GreeterTests.Rejects_blank_names(name: "")',
    method: "Rejects_blank_names(System.String)",
    line: 10,
    state: "failed",
  }),
  test("c", {
    displayName: 'N.GreeterTests.Rejects_blank_names(name: " ")',
    method: "Rejects_blank_names(System.String)",
    line: 10,
    state: "passed",
  }),
];

describe("unit tests model", () => {
  it("groups tests by namespace, class and data-driven method", () => {
    const [p] = buildTree([project(xunit)]);
    expect(p!.name).toBe("T.Tests");
    const [ns] = p!.children;
    expect(ns!.name).toBe("N");
    const [cls] = ns!.children;
    expect(cls!.name).toBe("GreeterTests");
    expect(cls!.children.map((c) => [c.kind, c.name])).toEqual([
      ["test", "Greets_by_name"],
      ["method", "Rejects_blank_names"],
    ]);
    expect(cls!.children[1]!.children.map((c) => c.name)).toEqual(['(name: " ")', '(name: "")']);
    expect(nodeStatus(cls!)).toBe("failed");
    expect(nodeStatus(cls!.children[0]!)).toBe("passed");
  });

  it("filters by name and by failure", () => {
    expect(buildTree([project(xunit)], "greets")[0]!.children[0]!.children[0]!.children).toHaveLength(1);
    const failed = buildTree([project(xunit)], "", true)[0]!.children[0]!.children[0]!.children;
    expect(failed.map((c) => [c.kind, c.name])).toEqual([["test", 'Rejects_blank_names(name: "")']]);
    expect(buildTree([project(xunit)], "nothing")).toEqual([]);
  });

  it("selects whole projects or the tests under nodes", () => {
    const [p] = buildTree([project(xunit)]);
    expect(selectionOf([p!])).toEqual([{ project: P, uids: [] }]);
    const method = p!.children[0]!.children[0]!.children[1]!;
    expect(selectionOf([method])).toEqual([{ project: P, uids: ["c", "b"] }]);
  });

  it("keeps discovered locations when a run update has none", () => {
    const merged = mergeTest(xunit[0], {
      ...xunit[0]!,
      file: null,
      line: null,
      state: "failed",
    });
    expect([merged.file, merged.line, merged.state]).toEqual(["/s/tests/T.Tests/UnitTest1.cs", 7, "failed"]);
  });

  it("counts results", () => {
    expect(countTests([project(xunit)])).toEqual({
      total: 3,
      passed: 2,
      failed: 1,
      skipped: 0,
    });
  });

  it("finds source locations in stack traces", () => {
    const [a, b] = stackLocations("   at N.GreeterTests.Fails() in /s/T.cs:line 12\n   at System.Foo()");
    expect([a!.path, a!.line]).toEqual(["/s/T.cs", 12]);
    expect(b!.path).toBeNull();
  });

  it("places gutter icons on test lines and the class declaration", () => {
    const lines = [
      "namespace N;",
      "",
      "public class GreeterTests",
      "{",
      "",
      "",
      "    [Fact]",
      "    public void Greets_by_name() {}",
      "",
      "    [Theory]",
    ];
    const targets = gutterTargets(
      lines,
      xunit.map((t) => ({ project: P, test: t })),
    );
    expect(targets.map((t) => [t.line, t.kind, t.name, t.tests.length])).toEqual([
      [3, "class", "GreeterTests", 3],
      [7, "test", "Greets_by_name", 1],
      [10, "test", "Rejects_blank_names", 2],
    ]);
  });
});
