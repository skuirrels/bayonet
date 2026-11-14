/**
 * Settings › Debugger › Data Views › "Show value tooltip on mouse hover":
 * while suspended, hovering an expression (or the selection under the mouse)
 * evaluates it in the selected frame.
 */
import type { EditorState, Extension } from "@codemirror/state";
import type { DebugSessionId, Variable } from "@/bindings";
import { hoverTooltip, ViewPlugin, type EditorView, type ViewUpdate } from "@codemirror/view";
import { showHint } from "@/editor/lsp/hint";
import { hoverPreemptions } from "@/editor/api";
import { workflowSettings } from "@/platform/workflowSettings";
import { navigationModifierHeld } from "@/platform/navigationModifier";
import { activeSession, evaluate, useDebug, type DebugSession } from "../state";
import { inlineValue } from "../values";
import { tx } from "@/platform/i18n";
import { iconMarkup, selectableText } from "@/ui";
import { mountValueTree } from "../ui/ValueHintTree";
import styles from "./editor.module.css";

const BASE_HOVER = 100;
const IDENT = /[\p{L}\p{N}_$]/u;

/**
 * An index that is safe to evaluate on hover: a number, a string literal or a
 * variable path. Never a call or an assignment, which could change the program.
 */
const SAFE_INDEX = /^\s*(?:\d+|"[^"\\]*"|'[^'\\]'|[\p{L}_$][\p{L}\p{N}_$]*(?:\.[\p{L}_$][\p{L}\p{N}_$]*)*)\s*$/u;

/** Start of the `a.b[i].c` path that ends at `end` in `t`, or `null` when an index is not safe. */
function pathStart(t: string, end: number): number | null {
  let start = end;
  for (;;) {
    const before = t[start - 1];
    if (before && IDENT.test(before)) start--;
    else if (before === "." && (IDENT.test(t[start - 2] ?? "") || t[start - 2] === "]")) start--;
    else if (before === "]") {
      const open = t.lastIndexOf("[", start - 2);
      if (open < 1 || !SAFE_INDEX.test(t.slice(open + 1, start - 1)) || !IDENT.test(t[open - 1]!)) return null;
      start = open;
    } else break;
  }
  return /^[\p{L}_$]/u.test(t[start] ?? "") ? start : null;
}

/**
 * The expression to show a value for at `pos`: the identifier with its `a.b.c` member
 * access and `a[0]` / `a.b[i]` indexers, an indexer when over its brackets or index,
 * or the selection containing `pos`.
 */
export function expressionAt(view: EditorView, pos: number): { from: number; to: number; text: string } | null {
  const { state } = view;
  const sel = state.selection.main;
  if (!sel.empty && pos >= sel.from && pos <= sel.to) {
    const text = state.sliceDoc(sel.from, sel.to).trim();
    return text && !text.includes("\n") ? { from: sel.from, to: sel.to, text } : null;
  }
  const line = state.doc.lineAt(pos);
  const t = line.text;
  const at = pos - line.from;
  const found = (start: number | null, end: number) =>
    start == null || start >= end ? null : { from: line.from + start, to: line.from + end, text: t.slice(start, end) };
  let end = at;
  while (end < t.length && IDENT.test(t[end]!)) end++;
  let start = at;
  while (start > 0 && IDENT.test(t[start - 1]!)) start--;
  const word = t.slice(start, end);
  const inIndex = () => {
    // Over `[`, `]` or the index itself: the element.
    const open = t.lastIndexOf("[", t[at] === "[" ? at : at - 1);
    const close = t.indexOf("]", t[at] === "]" ? at : Math.max(at, open + 1));
    if (open < 1 || close < 0 || t.slice(open + 1, close).includes("[")) return null;
    if (!SAFE_INDEX.test(t.slice(open + 1, close)) || !IDENT.test(t[open - 1]!)) return null;
    return found(pathStart(t, open), close + 1);
  };
  // Text inside a string literal is not a name: only an indexer around it is.
  const inString = (t.slice(0, start).replace(/\\./g, "").match(/"/g)?.length ?? 0) % 2 === 1;
  if (!word || /^\d/.test(word) || inString) return inIndex();
  return found(pathStart(t, end), end);
}

/** IDEA's value hint: `expression = value {type}`, objects expandable into a variables tree in place. */
export function valueHint(
  expression: string,
  value: Variable,
  session: DebugSessionId,
  max: number | null = 400,
): { dom: HTMLElement; destroy: () => void } {
  const dom = document.createElement("div");
  dom.className = `${styles.valueTooltip} ${selectableText}`;
  let tree: { unmount(): void } | null = null;
  if (value.variablesReference > 0) {
    // IDEA's value hint: the "+" expands the object into a variables tree in place.
    const expand = document.createElement("button");
    expand.type = "button";
    expand.className = styles.valueExpand!;
    expand.setAttribute("aria-label", tx("Expand value"));
    expand.setAttribute("aria-expanded", "false");
    expand.innerHTML = iconMarkup("chevron-right", 12) ?? "";
    expand.addEventListener("mousedown", (e) => e.preventDefault());
    expand.addEventListener("click", () => {
      if (tree) return;
      expand.remove();
      dom.classList.replace(selectableText, styles.valueTooltipTree!);
      dom.replaceChildren();
      tree = mountValueTree(dom, expression, value, session);
    });
    dom.append(expand);
  }
  const name = document.createElement("span");
  name.className = styles.valueName!;
  name.textContent = `${expression} = `;
  dom.append(name, max === null ? value.value : inlineValue(value.value, max));
  if (value.typeName) {
    const type = document.createElement("span");
    type.className = styles.valueType!;
    type.textContent = `  {${value.typeName}}`;
    dom.append(type);
  }
  return { dom, destroy: () => tree?.unmount() };
}

/** Hover evaluations of the current stop, shared by the value tooltip and its claim on the hover. */
let hovered: { stop: string; values: Map<string, Promise<Variable | null>> } = { stop: "", values: new Map() };

/** The value tooltip's expression at `pos` and its value, or `null` when there is none to show. */
function hoverValue(
  view: EditorView,
  pos: number,
): { expr: { from: number; to: number; text: string }; session: DebugSession; value: Promise<Variable | null> } | null {
  if (!workflowSettings().debugger.valueTooltip || navigationModifierHeld()) return null;
  const session = activeSession();
  if (!session || session.status !== "paused") return null;
  const expr = expressionAt(view, pos);
  if (!expr) return null;
  const stop = `${session.id}:${useDebug.getState().stopSeq}:${session.selectedFrame}`;
  if (hovered.stop !== stop) hovered = { stop, values: new Map() };
  let value = hovered.values.get(expr.text);
  if (!value) {
    value = evaluate(expr.text, "hover", session).catch(() => null);
    hovered.values.set(expr.text, value);
  }
  return { expr, session, value };
}

/**
 * While paused, a value tooltip wins over the documentation hover of the same spot:
 * the editor asks this before showing its own popup.
 */
async function valueTipClaims(view: EditorView, pos: number): Promise<boolean> {
  const hover = hoverValue(view, pos);
  return !!hover && (await hover.value) != null;
}

export function valueTooltip(): Extension {
  return [hoverPreemptions.of(valueTipClaims), valueTooltipSource()];
}

function valueTooltipSource(): Extension {
  return hoverTooltip(
    async (view, pos) => {
      const dbg = workflowSettings().debugger;
      // ⌘/Ctrl-hover shows the navigation info instead (closed on press by the LSP hover).
      const hover = hoverValue(view, pos);
      if (!hover) return null;
      const { expr, session } = hover;
      if (dbg.valueTooltipDelay > BASE_HOVER)
        await new Promise((r) => setTimeout(r, dbg.valueTooltipDelay - BASE_HOVER));
      const value = await hover.value;
      if (!value || navigationModifierHeld()) return null;
      return {
        pos: expr.from,
        end: expr.to,
        above: true,
        create: () => {
          return valueHint(expr.text, value, session.id);
        },
      };
    },
    { hoverTime: BASE_HOVER },
  );
}

const SELECTION_DELAY = 400;
const MAX_SELECTION = 200;

/** The selected text when it can be evaluated: one non-blank line, not too long. */
export function selectionExpression(state: EditorState): { from: number; text: string } | null {
  const sel = state.selection;
  if (sel.ranges.length !== 1 || sel.main.empty) return null;
  const text = state.sliceDoc(sel.main.from, sel.main.to).trim();
  if (!text || text.length > MAX_SELECTION || text.includes("\n")) return null;
  return { from: sel.main.from, text };
}

/**
 * Settings › Debugger › Data Views › "Show value tooltip on code selection": while
 * suspended, selecting an expression shows its value once the selection settles.
 */
export function selectionValueTooltip(): Extension {
  return ViewPlugin.fromClass(
    class {
      private timer: ReturnType<typeof setTimeout> | undefined;
      private seq = 0;
      constructor(readonly view: EditorView) {}

      update(u: ViewUpdate) {
        if (!u.selectionSet) return;
        clearTimeout(this.timer);
        this.seq++;
        if (!u.transactions.some((tr) => tr.isUserEvent("select"))) return;
        const seq = this.seq;
        this.timer = setTimeout(() => void this.show(seq), SELECTION_DELAY);
      }

      private async show(seq: number) {
        if (!workflowSettings().debugger.valueTooltipOnSelection) return;
        const session = activeSession();
        if (!session || session.status !== "paused") return;
        const expr = selectionExpression(this.view.state);
        if (!expr) return;
        let value;
        try {
          value = await evaluate(expr.text, "hover", session);
        } catch {
          return;
        }
        // The selection moved on while evaluating.
        if (seq !== this.seq) return;
        showHint(this.view, () => valueHint(expr.text, value, session.id), { pos: expr.from, timeout: 60_000 });
      }

      destroy() {
        clearTimeout(this.timer);
      }
    },
  );
}
