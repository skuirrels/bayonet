/**
 * Gutter ▶ icons on .NET test methods and their classes, from the tests the
 * Unit Tests window discovered (their file and line), coloured by the last
 * result. Clicking one runs those tests.
 */
import { RangeSet, StateEffect, type EditorState, type Extension } from "@codemirror/state";
import { EditorView, GutterMarker, ViewPlugin, gutter } from "@codemirror/view";
import type { TestNode, TestSelection } from "@/bindings";
import type { EditorDocContext } from "@/editor/api";
import { isVirtualUri } from "@/platform/virtualDocs";
import { tx } from "@/platform/i18n";
import { gutterIconMarkup, showContextMenu, type MenuItem } from "@/ui";
import styles from "@/features/run/Run.module.css";
import { statusOf } from "./model";
import { runTests, testsIn, useUnitTests } from "./store";

const DOTNET_SOURCE = /\.(cs|vb|fs)$/i;

export interface GutterTarget {
  /** 1-based line. */
  line: number;
  kind: "test" | "class";
  name: string;
  project: string;
  tests: TestNode[];
}

/** Worst status first: a failed row makes its method's icon red. */
function iconOf(target: GutterTarget): string {
  const states = target.tests.map((t) => statusOf(t.state));
  if (states.includes("failed")) return "gutter-run-failed";
  if (states.length && states.every((s) => s === "passed" || s === "skipped") && states.includes("passed"))
    return "gutter-run-passed";
  return target.kind === "class" ? "gutter-run-all" : "gutter-run";
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Markers for `file`: one per test line (data-driven rows share their method's
 * line), and one on each test class's declaration found in `lines`.
 */
export function gutterTargets(
  lines: readonly string[],
  found: ReadonlyArray<{ project: string; test: TestNode }>,
): GutterTarget[] {
  const byLine = new Map<number, GutterTarget>();
  const byType = new Map<string, { project: string; tests: TestNode[] }>();
  for (const { project, test } of found) {
    if (test.line) {
      const t = byLine.get(test.line);
      if (t) t.tests.push(test);
      else
        byLine.set(test.line, {
          line: test.line,
          kind: "test",
          name: (test.method ?? test.displayName).split("(")[0]!,
          project,
          tests: [test],
        });
    }
    if (test.typeName) {
      const e = byType.get(test.typeName) ?? { project, tests: [] };
      e.tests.push(test);
      byType.set(test.typeName, e);
    }
  }
  for (const [type, e] of byType) {
    const simple = type.slice(type.lastIndexOf(".") + 1).replace(/`\d+$/, "");
    const re = new RegExp(`\\b(?:class|record|struct|type|Class|Module)\\s+${escape(simple)}\\b`);
    const at = lines.findIndex((l) => re.test(l));
    if (at >= 0 && !byLine.has(at + 1))
      byLine.set(at + 1, {
        line: at + 1,
        kind: "class",
        name: simple,
        project: e.project,
        tests: e.tests,
      });
  }
  return [...byLine.values()].sort((a, b) => a.line - b.line);
}

function selectionOf(t: GutterTarget): TestSelection[] {
  return [{ project: t.project, uids: t.tests.map((x) => x.uid) }];
}

class TestMarker extends GutterMarker {
  constructor(readonly target: GutterTarget) {
    super();
  }
  override eq(other: GutterMarker): boolean {
    return (
      other instanceof TestMarker &&
      other.target.line === this.target.line &&
      iconOf(other.target) === iconOf(this.target) &&
      other.target.name === this.target.name
    );
  }
  override toDOM(): Node {
    const el = document.createElement("div");
    el.className = styles.runMarker ?? "";
    el.title = tx`Run '${this.target.name}'`;
    el.setAttribute("role", "button");
    el.setAttribute("aria-label", el.title);
    el.innerHTML = gutterIconMarkup(iconOf(this.target)) ?? "";
    return el;
  }
}

const refresh = StateEffect.define<null>();

function targetsOf(state: EditorState, path: string): GutterTarget[] {
  const lines: string[] = [];
  for (let i = 1; i <= state.doc.lines; i++) lines.push(state.doc.line(i).text);
  return gutterTargets(lines, testsIn(path)).filter((t) => t.line <= state.doc.lines);
}

export function testGutter(ctx: EditorDocContext): Extension {
  const path = ctx.path;
  if (isVirtualUri(path) || !DOTNET_SOURCE.test(path)) return [];
  let cache: {
    version: number;
    doc: unknown;
    targets: GutterTarget[];
    set: RangeSet<GutterMarker>;
  } | null = null;
  const compute = (state: EditorState) => {
    const version = useUnitTests.getState().version;
    if (cache && cache.version === version && cache.doc === state.doc) return cache;
    const targets = targetsOf(state, path);
    const set = RangeSet.of(
      targets.map((t) => new TestMarker(t).range(state.doc.line(t.line).from)),
      true,
    );
    cache = { version, doc: state.doc, targets, set };
    return cache;
  };
  const watcher = ViewPlugin.fromClass(
    class {
      readonly unsubscribe: () => void;
      constructor(view: EditorView) {
        this.unsubscribe = useUnitTests.subscribe((s, prev) => {
          if (s.version !== prev.version) view.dispatch({ effects: refresh.of(null) });
        });
      }
      destroy() {
        this.unsubscribe();
      }
    },
  );
  return [
    watcher,
    gutter({
      class: `${styles.runGutter} cm-openidea-gutter-run`,
      markers: (view) => compute(view.state).set,
      domEventHandlers: {
        mousedown: (view, block, event) => {
          if ((event as MouseEvent).button !== 0) return false;
          const line = view.state.doc.lineAt(block.from).number;
          const target = compute(view.state).targets.find((t) => t.line === line);
          if (!target) return false;
          const rect = (event.target as HTMLElement).getBoundingClientRect();
          event.preventDefault();
          const busy = !!useUnitTests.getState().busy;
          const items: MenuItem[] = [
            {
              label: tx`Run '${target.name}'`,
              verbatim: true,
              icon: "run",
              disabled: busy,
              run: () => void runTests(selectionOf(target)),
            },
          ];
          showContextMenu({ x: rect.left, y: rect.bottom + 2 }, items);
          return true;
        },
      },
    }),
  ];
}
