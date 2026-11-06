import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import type { FlameNode, TraceProfile } from "@/bindings";
import { buildHeat, lineTimeText } from "./heatGutter";
import {
  butterfly,
  displayFrame,
  formatMs,
  formatShare,
  gcSummary,
  heatByFile,
  lineHeatByFile,
  heatLevel,
  initialCallPath,
  methodTable,
  ROOT_CALLER,
} from "./trace";

const node = (name: string, totalMs: number, selfMs: number, children: FlameNode[] = []): FlameNode => ({
  name,
  totalMs,
  selfMs,
  children,
});

// Main → Work → Fib → Fib (recursion); Main → Fib; Idle on its own.
const root = node("All threads", 100, 0, [
  node("A!N.Program.Main()", 70, 5, [
    node("A!N.Worker.Work()", 40, 10, [
      node("A!N.Program.Fib(int32)", 30, 20, [node("A!N.Program.Fib(int32)", 10, 10)]),
    ]),
    node("A!N.Program.Fib(int32)", 25, 25),
  ]),
  node("System!Idle", 30, 30),
]);

describe("CPU trace methods", () => {
  it("adds up each method's self and total time, counting recursion once", () => {
    expect(methodTable(root).map((m) => [m.name, m.selfMs, m.totalMs, m.paths])).toEqual([
      ["A!N.Program.Fib(int32)", 55, 55, 3],
      ["System!Idle", 30, 30, 1],
      ["A!N.Worker.Work()", 10, 40, 1],
      ["A!N.Program.Main()", 5, 70, 1],
    ]);
  });

  it("finds a method's callers and callees", () => {
    const b = butterfly(root, "A!N.Program.Fib(int32)");
    expect([b.selfMs, b.totalMs]).toEqual([55, 55]);
    expect(b.callers).toEqual([
      { name: "A!N.Worker.Work()", ms: 30 },
      { name: "A!N.Program.Main()", ms: 25 },
    ]);
    // The recursive call is not a callee.
    expect(b.callees).toEqual([]);
    const main = butterfly(root, "A!N.Program.Main()");
    expect(main.callers).toEqual([{ name: ROOT_CALLER, ms: 70 }]);
    expect(main.callees.map((c) => [c.name, c.ms])).toEqual([
      ["A!N.Worker.Work()", 40],
      ["A!N.Program.Fib(int32)", 25],
    ]);
  });

  it("opens the call tree where your own code spends its time", () => {
    // The idle thread is the hottest path; Main's work is where the own time is.
    const tree = node("All threads", 100, 0, [
      node("System!Wait", 60, 0, [node("UNMANAGED_CODE_TIME", 60, 60)]),
      node("A!N.Program.Main()", 40, 0, [
        node("System!Run", 30, 0, [node("A!N.Program.Spin()", 30, 30)]),
        node("A!N.Program.Log()", 10, 10),
      ]),
    ]);
    const own = (name: string) => name.startsWith("A!");
    expect([...initialCallPath(tree, own)]).toEqual(["1", "1/0"]);
    // Without any own code it follows the hottest frames.
    expect([...initialCallPath(tree, () => false)]).toEqual(["0"]);
  });

  it("names frames the way the code reads", () => {
    const show = (name: string) => {
      const d = displayFrame(name);
      return [d.method, d.namespace, d.module];
    };
    expect(show("Hot!Hot.Work.Calculator.Spin()")).toEqual(["Calculator.Spin", "Hot.Work", "Hot"]);
    expect(show("Hot!Hot.Work.Worker+<RunAsync>d__0.MoveNext()")).toEqual([
      "Worker.RunAsync (async)",
      "Hot.Work",
      "Hot",
    ]);
    expect(show("Hot!Hot.Work.Calculator+Nested..ctor()")).toEqual(["Calculator.Nested..ctor", "Hot.Work", "Hot"]);
    expect(show("Hot!Hot.Work.Calculator+Nested.Deep()")).toEqual(["Calculator.Nested.Deep", "Hot.Work", "Hot"]);
    expect(show("Hot!Hot.Work.Generic`1[System.__Canon].Build(!0)")).toEqual(["Generic.Build", "Hot.Work", "Hot"]);
    expect(show("Hot!Hot.Work.Program+<>c.<Main>b__0_0(int32)")).toEqual(["Program.Main (lambda)", "Hot.Work", "Hot"]);
    expect(show("Hot!Hot.Work.Program.<Main>g__Local|0_1(int32)")).toEqual([
      "Program.Local (local function in Main)",
      "Hot.Work",
      "Hot",
    ]);
    expect(show("App!Program.Run()")).toEqual(["Program.Run", "", "App"]);
    expect(show("UNMANAGED_CODE_TIME")).toEqual(["UNMANAGED_CODE_TIME", "", ""]);
    expect(displayFrame("A!N.T.M(int32,class System.String)").args).toBe("(int32,class System.String)");
  });

  it("formats times and shares to three significant figures", () => {
    expect([formatMs(4), formatMs(86.34), formatMs(412), formatMs(1240), formatMs(12_345), formatMs(123_456)]).toEqual([
      "4 ms",
      "86.3 ms",
      "412 ms",
      "1.24 s",
      "12.3 s",
      "123 s",
    ]);
    expect([formatShare(0.182), formatShare(0.004), formatShare(0.0004), formatShare(0)]).toEqual([
      "18 %",
      "0.4 %",
      "< 0.1 %",
      "0.0 %",
    ]);
  });
});

describe("editor heat", () => {
  const trace: TraceProfile = {
    path: "/d/1-2.speedscope.json",
    durationMs: 1000,
    threads: 2,
    root,
    lines: [],
    collections: [],
    sources: [
      { frame: "A!N.Program.Main()", path: "/src/Program.cs", line: 4, col: 23, startLine: 4, endLine: 9 },
      { frame: "A!N.Program.Fib(int32)", path: "/src/Program.cs", line: 11, col: 15, startLine: 11, endLine: 11 },
      { frame: "A!N.Worker.Work()", path: "/src/Worker.cs", line: 2, col: 16, startLine: 2, endLine: 5 },
      // Unsampled frame names are ignored.
      { frame: "A!N.Gone()", path: "/src/Gone.cs", line: 0, col: 0, startLine: 0, endLine: 0 },
    ],
  };

  it("grades methods by their share of thread time", () => {
    expect([0, 0.004, 0.005, 0.02, 0.05, 0.1, 0.3].map(heatLevel)).toEqual([0, 0, 1, 2, 3, 4, 5]);
  });

  it("collects each file's methods with their time", () => {
    const heat = heatByFile(trace);
    expect(Object.keys(heat).sort()).toEqual(["/src/Program.cs", "/src/Worker.cs"]);
    expect(heat["/src/Program.cs"]!.map((m) => [m.line, m.selfMs, m.totalMs, m.level])).toEqual([
      [4, 5, 70, 5],
      [11, 55, 55, 5],
    ]);
    expect(heat["/src/Worker.cs"]![0]!.totalShare).toBeCloseTo(0.4);
  });

  it("adds frames of one declaration together", () => {
    const merged = heatByFile({
      ...trace,
      sources: [
        { frame: "A!N.Program.Main()", path: "/p.cs", line: 4, col: 2, startLine: 4, endLine: 9 },
        { frame: "A!N.Worker.Work()", path: "/p.cs", line: 4, col: 2, startLine: 4, endLine: 9 },
      ],
    });
    expect(merged["/p.cs"]!.map((m) => [m.selfMs, m.totalMs])).toEqual([[15, 70]]);
  });

  it("puts a lens above each method and a bar over its lines", () => {
    const doc = Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n");
    const state = EditorState.create({ doc });
    const revealed: string[] = [];
    const h = buildHeat(state, heatByFile(trace)["/src/Program.cs"]!, [], (f) => revealed.push(f));
    expect(h.lenses.map((l) => [state.doc.lineAt(l.pos).number, l.title])).toEqual([
      [5, "70 % · 70 ms · self 5 ms"],
      [12, "55 % · 55 ms · self 55 ms"],
    ]);
    // Main spans lines 5 to 10, Fib line 12.
    const lines: number[] = [];
    h.markers.between(0, state.doc.length, (from) => {
      lines.push(state.doc.lineAt(from).number);
    });
    expect(lines).toEqual([5, 6, 7, 8, 9, 10, 12]);
    h.lenses[1]!.run!(null as never, null as never);
    expect(revealed).toEqual(["A!N.Program.Fib(int32)"]);
  });

  const withLines: TraceProfile = {
    ...trace,
    lines: [
      { path: "/src/Program.cs", line: 6, selfMs: 2, totalMs: 60 },
      { path: "/src/Program.cs", line: 11, selfMs: 55, totalMs: 55 },
      // Under 0.5 % of thread time: not marked.
      { path: "/src/Program.cs", line: 7, selfMs: 0.2, totalMs: 0.2 },
    ],
  };

  it("marks hot source lines with their own bar and time when the trace has lines", () => {
    const lines = lineHeatByFile(withLines)["/src/Program.cs"]!;
    expect(lines.map((l) => [l.line, l.level])).toEqual([
      [6, 5],
      [11, 5],
    ]);
    expect(lines.map(lineTimeText)).toEqual(["60 ms · self 2 ms", "55 ms"]);
    const doc = Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n");
    const state = EditorState.create({ doc });
    const h = buildHeat(state, heatByFile(trace)["/src/Program.cs"]!, lines, () => {});
    // Bars on the hot lines only, not over whole methods; lenses stay.
    const marked: number[] = [];
    h.markers.between(0, state.doc.length, (from) => {
      marked.push(state.doc.lineAt(from).number);
    });
    expect(marked).toEqual([7, 12]);
    expect(h.lenses).toHaveLength(2);
    const at: number[] = [];
    h.times.between(0, state.doc.length, (from) => {
      at.push(from);
    });
    expect(at).toEqual([state.doc.line(7).to, state.doc.line(12).to]);
  });

  it("sums up garbage collections", () => {
    expect(gcSummary(trace)).toBeNull();
    const g = gcSummary({
      ...trace,
      collections: [
        {
          startMs: 1,
          pauseMs: 4,
          generation: 0,
          reason: "AllocSmall",
          kind: "NonConcurrentGC",
          heapBeforeMb: 8,
          heapAfterMb: 1,
        },
        {
          startMs: 9,
          pauseMs: 16,
          generation: 2,
          reason: "Induced",
          kind: "BackgroundGC",
          heapBeforeMb: 9,
          heapAfterMb: 2,
        },
      ],
    })!;
    expect([g.count, g.byGeneration, g.pauseMs, g.pauseShare, g.longestPauseMs]).toEqual([2, [1, 0, 1], 20, 0.02, 16]);
  });
});
