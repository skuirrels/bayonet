/** DBG-6 data tips: what a hover evaluates, and the value tooltip winning over the documentation hover. */
import { afterEach, describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { Variable } from "@/bindings";
import { CONTRIBUTED_HOVER, type ContributedHover } from "@/editor/api";

const evaluated: string[] = [];

vi.mock("@/platform/ipc", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/platform/ipc")>();
  return {
    ...real,
    commands: {
      ...real.commands,
      debugEvaluateIn: (_s: number, _frame: number | null, expression: string) => {
        evaluated.push(expression);
        const v: Variable = {
          name: expression,
          value: "12.5",
          typeName: "decimal",
          variablesReference: 0,
          evaluateName: expression,
        };
        return Promise.resolve(
          expression === "missing"
            ? { status: "error" as const, error: { kind: "invalidInput", message: "no such name" } }
            : { status: "ok" as const, data: v },
        );
      },
    },
  };
});

import { hoverSource } from "@/editor/lsp/hover";
import { useDebug, type DebugSession } from "../state";
import { expressionAt, valueTooltip } from "../editor/valueTooltip";

const session: DebugSession = {
  id: 1,
  name: "main",
  adapter: "coreclr",
  parent: null,
  launch: null,
  process: null,
  status: "paused",
  stoppedThread: 1,
  stopReason: "breakpoint",
  stopDescription: null,
  threads: [{ id: 1, name: "Main" }],
  selectedThread: 1,
  frames: [],
  selectedFrame: 7,
  hits: 1,
  exitCode: null,
  caps: null,
};

const docHover: ContributedHover = (state, pos) => {
  const w = state.wordAt(pos);
  return w ? { from: w.from, to: w.to, markdown: "decimal Order.Total" } : null;
};

let view: EditorView | null = null;
function editor(doc: string, debuggerTip = false) {
  const state = EditorState.create({
    doc,
    extensions: [
      EditorState.languageData.of(() => [{ [CONTRIBUTED_HOVER]: docHover }]),
      debuggerTip ? valueTooltip() : [],
    ],
  });
  view = new EditorView({ state, parent: document.body.appendChild(document.createElement("div")) });
  return view;
}

afterEach(() => {
  view?.destroy();
  view = null;
  document.body.replaceChildren();
  evaluated.length = 0;
  useDebug.setState({ sessions: {}, active: null });
});

const at = (doc: string, marker: string, offset = 0) => {
  const v = editor(doc);
  return expressionAt(v, doc.indexOf(marker) + offset)?.text ?? null;
};

describe("data tip expressions", () => {
  it("takes member access and indexers, never calls or assignments", () => {
    const line = 'var t = orders[0].Total + a.b[i].c + dict["k"] + Get().Total + m[f(x)] + q[j = 1];';
    expect(at(line, "Total")).toBe("orders[0].Total");
    expect(at(line, "orders")).toBe("orders");
    expect(at(line, "[0]", 1)).toBe("orders[0]");
    expect(at(line, "].Total")).toBe("orders[0]");
    expect(at(line, ".c", 1)).toBe("a.b[i].c");
    expect(at(line, "i]")).toBe("i");
    expect(at(line, '"k"', 1)).toBe('dict["k"]');
    expect(at(line, "Get().Total", 7)).toBe("Total");
    expect(at(line, ")] +", 1)).toBeNull();
    expect(at(line, "1]", 1)).toBeNull();
    expect(at("this.count + 1", "count")).toBe("this.count");
    expect(at("x + 42", "42")).toBeNull();
  });
});

describe("value tooltip over the documentation hover", () => {
  it("the documentation hover stays out where a value is shown while paused", async () => {
    const doc = "var t = order.Total;";
    const v = editor(doc, true);
    const pos = doc.indexOf("Total") + 1;
    // Not debugging: documentation as usual.
    expect(await hoverSource(v, pos)).not.toBeNull();
    useDebug.setState({ sessions: { 1: session }, active: 1, stopSeq: 3 });
    expect(await hoverSource(v, pos)).toBeNull();
    expect(evaluated).toEqual(["order.Total"]);
  });

  it("keeps the documentation when the debugger has no value there", async () => {
    const doc = "missing";
    const v = editor(doc, true);
    useDebug.setState({ sessions: { 1: session }, active: 1, stopSeq: 3 });
    expect(await hoverSource(v, 2)).not.toBeNull();
  });
});
