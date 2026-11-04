import { describe, expect, it } from "vitest";
import type { FlameNode, MetricSample } from "@/bindings";
import {
  addSamples,
  artifactInfo,
  formatBytes,
  formatValue,
  frameCategory,
  groupStacks,
  headlineFrame,
  heapDiff,
  layoutFlame,
  linePath,
  niceMax,
  nodeAt,
  orderSeries,
  processGroup,
  ruleFires,
  shortMethod,
  type TriggerRule,
} from "./model";

const sample = (name: string, time: number, value: number, over: Partial<MetricSample> = {}): MetricSample => ({
  time,
  provider: "System.Runtime",
  name,
  displayName: name,
  unit: "%",
  counterType: "Metric",
  tags: "",
  value,
  ...over,
});

const node = (name: string, totalMs: number, children: FlameNode[] = []): FlameNode => ({
  name,
  totalMs,
  selfMs: totalMs - children.reduce((a, c) => a + c.totalMs, 0),
  children,
});

describe("live counters", () => {
  it("keeps a capped series per counter and tag set", () => {
    let s = addSamples({}, [sample("cpu-usage", 1000, 1), sample("gen", 1000, 2, { tags: "gen=0" })]);
    s = addSamples(
      s,
      [sample("cpu-usage", 2000, 3), sample("cpu-usage", 3000, 4), sample("x", 3000, null as never)],
      2,
    );
    expect(Object.keys(s)).toEqual(["System.Runtime/cpu-usage", "System.Runtime/gen[gen=0]"]);
    expect(s["System.Runtime/cpu-usage"]!.points).toEqual([
      { t: 2000, v: 3 },
      { t: 3000, v: 4 },
    ]);
    expect(s["System.Runtime/gen[gen=0]"]!.displayName).toBe("gen (gen=0)");
  });

  it("puts the headline counters first", () => {
    const s = addSamples({}, [sample("alloc", 1, 1), sample("working-set", 1, 1), sample("cpu-usage", 1, 1)]);
    expect(orderSeries(s).map((x) => x.name)).toEqual(["cpu-usage", "working-set", "alloc"]);
  });

  it("formats values by unit", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatValue(12.345, "%")).toBe("12.3 %");
    expect(formatValue(68.4, "MB")).toBe("68.4 MB");
    expect(formatValue(2048, "MB")).toBe("2.00 GB");
    expect(formatValue(12_500, "count")).toBe("12.5k");
    expect(formatValue(0.004, "")).toBe("0.00");
    expect(niceMax(0.3)).toBe(0.5);
    expect(niceMax(68)).toBe(100);
    expect(niceMax(0)).toBe(1);
  });

  it("draws a line in the chart box", () => {
    const d = linePath(
      [
        { t: 0, v: 0 },
        { t: 10, v: 5 },
        { t: 20, v: 20 },
      ],
      100,
      50,
      0,
      20,
      10,
    );
    expect(d).toBe("M0.0,50.0L50.0,25.0L100.0,0.0");
  });
});

describe("thread stacks", () => {
  const frames = [
    { module: "", method: "[Native Frames]" },
    { module: "System.Private.CoreLib", method: "System.Threading.Monitor.Wait(class System.Object,int32)" },
    { module: "Fixture.Api", method: "Program.<Main>$(class System.String[])" },
  ];
  it("groups identical stacks and names them after user code", () => {
    const groups = groupStacks([
      { id: "a", frames: frames.slice(0, 1) },
      { id: "b", frames },
      { id: "c", frames },
    ]);
    expect(groups.map((g) => g.threads)).toEqual([["b", "c"], ["a"]]);
    expect(headlineFrame(frames, new Set(["Fixture.Api"]))?.module).toBe("Fixture.Api");
    expect(headlineFrame(frames, new Set())?.module).toBe("System.Private.CoreLib");
    expect(shortMethod(frames[1]!.method)).toBe("Monitor.Wait");
  });
});

describe("flame graph", () => {
  const root = node("All threads", 100, [node("A!Main", 60, [node("A!Work", 40)]), node("System!Idle", 40)]);

  it("lays out callers above callees across the width", () => {
    const cells = layoutFlame(root);
    expect(cells.map((c) => [c.node.name, c.depth, c.x, c.w])).toEqual([
      ["All threads", 0, 0, 1],
      ["A!Main", 1, 0, 0.6],
      ["A!Work", 2, 0, 0.4],
      ["System!Idle", 1, 0.6, 0.4],
    ]);
  });

  it("zooms into a node with its ancestors on top", () => {
    expect(nodeAt(root, "0/0")?.name).toBe("A!Work");
    const cells = layoutFlame(root, "0");
    expect(cells.map((c) => [c.node.name, c.depth, c.w, c.ancestor])).toEqual([
      ["All threads", 0, 1, true],
      ["A!Main", 1, 1, false],
      ["A!Work", 2, 40 / 60, false],
    ]);
    expect(layoutFlame(root, "", 0.5).map((c) => c.node.name)).toEqual(["All threads", "A!Main"]);
  });

  it("colours frames by where they come from", () => {
    const user = new Set(["A"]);
    expect([
      frameCategory("A!Main", user),
      frameCategory("System!Idle", user),
      frameCategory("Thread (7)", user),
    ]).toEqual(["user", "dotnet", "native"]);
  });
});

describe("heap snapshots", () => {
  it("compares two snapshots, growth first", () => {
    const before = [
      { name: "System.String", module: "c", bytes: 100, count: 10 },
      { name: "Gone", module: "c", bytes: 50, count: 1 },
    ];
    const after = [
      { name: "System.String", module: "c", bytes: 300, count: 25 },
      { name: "New", module: "c", bytes: 10, count: 1 },
    ];
    expect(heapDiff(before, after).map((r) => [r.name, r.deltaBytes, r.deltaCount])).toEqual([
      ["System.String", 200, 15],
      ["New", 10, 1],
      ["Gone", -50, -1],
    ]);
  });
});

describe("trigger rules", () => {
  const rule: TriggerRule = {
    id: "r",
    enabled: true,
    counter: "cpu-usage",
    op: ">",
    threshold: 50,
    seconds: 3,
    action: "trace",
    cooldownSeconds: 60,
  };
  const series = (values: number[]) =>
    addSamples(
      {},
      values.map((v, i) => sample("cpu-usage", i * 1000, v)),
    );

  it("fires once the condition held for the whole window", () => {
    expect(ruleFires(rule, series([10, 60, 70, 80, 90]), 4000)).toBe(true);
    expect(ruleFires(rule, series([10, 60, 70, 20, 90]), 4000)).toBe(false);
    // Not enough history yet.
    expect(ruleFires(rule, series([90, 90]), 1000)).toBe(false);
    // Cooling down, or off.
    expect(ruleFires(rule, series([90, 90, 90, 90, 90]), 4000, 1000)).toBe(false);
    expect(ruleFires({ ...rule, enabled: false }, series([90, 90, 90, 90, 90]), 4000)).toBe(false);
  });
});

it("reads the pid and time of a collected file", () => {
  expect(artifactInfo("/d/57544-1762245926123.gcdump")).toEqual({ pid: 57544, time: 1762245926123 });
  expect(artifactInfo("/d/other.dmp")).toEqual({ pid: null, time: null });
});

it("puts the project's processes first and tooling last", () => {
  const root = "/src/app";
  expect(processGroup({ name: "Fixture.Api", commandLine: "/src/app/bin/Fixture.Api" }, root)).toBe("project");
  expect(processGroup({ name: "dotnet", commandLine: "dotnet /src/app/bin/Api.dll" }, root)).toBe("project");
  expect(processGroup({ name: "Planner.Api", commandLine: "/other/Planner.Api" }, root)).toBe("apps");
  expect(processGroup({ name: "MSBuild", commandLine: "/usr/share/dotnet/sdk/10.0.400/MSBuild.dll" }, root)).toBe(
    "tooling",
  );
  expect(processGroup({ name: "dotnet", commandLine: "dotnet /sdk/Roslyn/bincore/VBCSCompiler.dll" }, root)).toBe(
    "tooling",
  );
});
