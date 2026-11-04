import { describe, expect, it } from "vitest";
import type { Binlog, BinlogProject, BinlogTarget } from "@/bindings";
import {
  ancestorKeys,
  childNodes,
  findBinlogNode,
  indexLog,
  rootNodes,
  searchLog,
  slowestTargets,
  timelineBar,
} from "./model";

const target = (name: string, over: Partial<BinlogTarget> = {}): BinlogTarget => ({
  name,
  file: null,
  parent: null,
  startMs: 0,
  durationMs: 10,
  succeeded: true,
  skipped: null,
  tasks: [],
  messages: [],
  ...over,
});

const project = (id: number, name: string, over: Partial<BinlogProject> = {}): BinlogProject => ({
  id,
  parent: null,
  file: `/s/${name}`,
  name,
  targetFramework: null,
  configuration: null,
  entryTargets: "",
  startMs: 0,
  durationMs: 100,
  succeeded: true,
  errors: 0,
  warnings: 0,
  targets: [],
  messages: [],
  ...over,
});

const log: Binlog = {
  formatVersion: 28,
  succeeded: false,
  cancelled: false,
  durationMs: 200,
  projects: [
    project(1, "App.slnx", { targets: [target("Build")] }),
    project(2, "Core.csproj", {
      parent: 1,
      targetFramework: "net10.0",
      targets: [
        target("PrepareForBuild", { skipped: "condition false" }),
        target("CoreCompile", {
          durationMs: 80,
          tasks: [
            {
              name: "Csc",
              startMs: 5,
              durationMs: 70,
              succeeded: false,
              commandLine: "csc /out:Core.dll",
              messages: [
                {
                  severity: "error",
                  text: "CS0103: x does not exist",
                  code: "CS0103",
                  file: "/s/A.cs",
                  line: 3,
                  col: 4,
                },
              ],
            },
          ],
        }),
      ],
    }),
  ],
  diagnostics: [
    { severity: "error", text: "CS0103: x does not exist", code: "CS0103", file: "/s/A.cs", line: 3, col: 4 },
  ],
  messages: [],
  truncated: false,
};

describe("binary log tree", () => {
  it("puts errors first and nests the projects a project built", () => {
    const ix = indexLog(log);
    const roots = rootNodes(ix);
    expect(roots.map((n) => n.key)).toEqual(["diagnostics", "p1"]);
    const children = childNodes(ix, roots[1]!)!;
    expect(children.map((n) => n.key)).toEqual(["p1/t0", "p2"]);
  });

  it("hides skipped targets unless asked, keeping their keys", () => {
    const core = findBinlogNode(indexLog(log), "p2")!;
    expect(childNodes(indexLog(log), core)!.map((n) => n.key)).toEqual(["p2/t1"]);
    expect(childNodes(indexLog(log, true), core)!.map((n) => n.key)).toEqual(["p2/t0", "p2/t1"]);
  });

  it("finds a node by key and lists the nodes above it", () => {
    const ix = indexLog(log);
    const key = "p2/t1/k0/m0";
    const node = findBinlogNode(ix, key);
    expect(node?.kind).toBe("message");
    expect(ancestorKeys(ix, key)).toEqual(["p1", "p2", "p2/t1", "p2/t1/k0"]);
    expect(ancestorKeys(ix, "diagnostics/m0")).toEqual(["diagnostics"]);
  });

  it("searches names, command lines and messages", () => {
    const ix = indexLog(log);
    expect(searchLog(ix, "csc").map((h) => h.key)).toEqual(["p2/t1/k0"]);
    expect(searchLog(ix, "CS0103").map((h) => [h.key, h.location])).toEqual([
      ["p2/t1/k0/m0", "Core.csproj › CoreCompile › Csc"],
    ]);
    expect(searchLog(ix, "  ")).toEqual([]);
  });

  it("ranks the slowest targets that ran", () => {
    expect(slowestTargets(log).map((t) => t.target.name)).toEqual(["CoreCompile", "Build"]);
  });

  it("places bars on the timeline", () => {
    expect(timelineBar(50, 100, 200)).toEqual({ left: 0.25, width: 0.5 });
    const end = timelineBar(190, 100, 200);
    expect(end.left).toBe(0.95);
    expect(end.width).toBeCloseTo(0.05);
    expect(timelineBar(0, 10, 0)).toEqual({ left: 0, width: 0 });
  });
});
