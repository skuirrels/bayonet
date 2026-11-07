/**
 * CPU trace heat in C# editors: a lens above each sampled method (`18 % · 412 ms
 * self`) and a bar in the change lane, coloured from warm to hot by share of all
 * thread time. Where the trace has source lines, each hot line gets its own bar and
 * its time at the end of the line; otherwise the bar covers the whole method.
 * Clicking a lens shows the method in the CPU Trace tab. Positions follow edits
 * until the next trace.
 */
import { RangeSet, StateEffect, StateField, type EditorState, type Extension } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType, type DecorationSet } from "@codemirror/view";
import { LaneMarker, codeLenses, gutterLaneMarkers, type CodeLens, type EditorDocContext } from "@/editor/api";
import { tx } from "@/platform/i18n";
import { shownHeat, shownLineHeat, useTraceHeat } from "./heat";
import { displayFrame, formatMs, formatShare, type LineHeat, type MethodHeat } from "./trace";

const CSHARP = /\.(cs|csx)$/i;

interface Heat {
  markers: RangeSet<GutterMarker>;
  lenses: readonly CodeLens[];
  times: DecorationSet;
}

const NONE: Heat = { markers: RangeSet.empty, lenses: [], times: Decoration.none };

export const heatColor = (level: number) => `var(--perf-heat-${level})`;

function tooltip(m: MethodHeat): string {
  const name = displayFrame(m.frame).method;
  const total = formatMs(m.totalMs);
  const self = formatMs(m.selfMs);
  const share = formatShare(m.totalShare);
  return tx`${name}: ${total} in total (${share} of thread time), ${self} in the method itself. Click to show it in the CPU trace.`;
}

function lineTooltip(l: LineHeat): string {
  const total = formatMs(l.totalMs);
  const self = formatMs(l.selfMs);
  const share = formatShare(l.totalShare);
  return tx`${total} on this line (${share} of thread time), ${self} running the line itself rather than what it calls.`;
}

/**
 * `1.38 s`, or `2.39 s · self 12 ms` for a line whose time is mostly in what it calls.
 * Self time under a millisecond is left out (the tooltip still has it).
 */
export function lineTimeText(l: LineHeat): string {
  const total = formatMs(l.totalMs);
  const showSelf = l.selfMs >= 1 && l.selfMs < l.totalMs * 0.95;
  return showSelf ? `${total} · ${tx`self`} ${formatMs(l.selfMs)}` : total;
}

class LineTime extends WidgetType {
  constructor(
    readonly text: string,
    readonly level: number,
    readonly title: string,
  ) {
    super();
  }

  eq(other: LineTime) {
    return other.text === this.text && other.level === this.level && other.title === this.title;
  }

  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-trace-line-time";
    el.style.setProperty("--trace-heat", heatColor(this.level));
    el.textContent = this.text;
    el.title = this.title;
    el.contentEditable = "false";
    return el;
  }

  ignoreEvent() {
    return false;
  }
}

/** Lenses, lane markers and line times for `heat` in `state`'s document. Exported for tests. */
export function buildHeat(
  state: EditorState,
  heat: readonly MethodHeat[],
  lines: readonly LineHeat[],
  reveal: (frame: string) => void,
): Heat {
  if (!heat.length && !lines.length) return NONE;
  const doc = state.doc;
  const markers = [];
  const lenses: CodeLens[] = [];
  for (const m of heat) {
    if (m.level === 0 || m.line >= doc.lines) continue;
    const line = doc.line(m.line + 1);
    const title = tooltip(m);
    lenses.push({
      pos: Math.min(line.from + m.col, line.to),
      icon: "heat",
      title: `${formatShare(m.totalShare)} · ${formatMs(m.totalMs)}${m.selfMs > 0 ? ` · ${tx`self`} ${formatMs(m.selfMs)}` : ""}`,
      tooltip: title,
      order: 50,
      run: () => reveal(m.frame),
    });
    if (lines.length) continue;
    const marker = new LaneMarker(heatColor(m.level), "bar", title);
    const last = Math.min(m.endLine, doc.lines - 1);
    for (let l = Math.max(m.startLine, 0); l <= last; l++) markers.push(marker.range(doc.line(l + 1).from));
  }
  const times = [];
  for (const l of lines) {
    if (l.line >= doc.lines) continue;
    const line = doc.line(l.line + 1);
    const title = lineTooltip(l);
    markers.push(new LaneMarker(heatColor(l.level), "bar", title).range(line.from));
    times.push(Decoration.widget({ widget: new LineTime(lineTimeText(l), l.level, title), side: 1 }).range(line.to));
  }
  return { markers: RangeSet.of(markers, true), lenses, times: Decoration.set(times, true) };
}

const setHeat = StateEffect.define<Heat>();

const heatField = StateField.define<Heat>({
  create: () => NONE,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setHeat)) return e.value;
    if (!tr.docChanged || value === NONE) return value;
    return {
      markers: value.markers.map(tr.changes),
      lenses: value.lenses.map((l) => ({ ...l, pos: tr.changes.mapPos(l.pos) })),
      times: value.times.map(tr.changes),
    };
  },
});

class HeatSync {
  private readonly unsub: () => void;
  private shown: readonly MethodHeat[] | null = null;
  private shownLines: readonly LineHeat[] | null = null;
  private destroyed = false;

  constructor(
    private readonly view: EditorView,
    private readonly path: string,
    private readonly reveal: (frame: string) => void,
  ) {
    this.unsub = useTraceHeat.subscribe(() => this.sync());
    // Not inside the constructor's update cycle.
    queueMicrotask(() => this.sync());
  }

  sync() {
    if (this.destroyed) return;
    const heat = shownHeat(this.path);
    const lines = shownLineHeat(this.path);
    if (heat === this.shown && lines === this.shownLines) return;
    this.shown = heat;
    this.shownLines = lines;
    this.view.dispatch({ effects: setHeat.of(buildHeat(this.view.state, heat, lines, this.reveal)) });
  }

  destroy() {
    this.destroyed = true;
    this.unsub();
  }
}

const theme = EditorView.baseTheme({
  ".cm-trace-line-time": {
    color: "var(--fg-inlay)",
    marginLeft: "3ch",
    display: "inline-block",
    whiteSpace: "nowrap",
    verticalAlign: "bottom",
    userSelect: "none",
    fontVariantNumeric: "tabular-nums",
  },
  ".cm-trace-line-time::before": {
    content: '""',
    display: "inline-block",
    width: "0.5em",
    height: "0.5em",
    marginRight: "0.5ch",
    borderRadius: "50%",
    background: "var(--trace-heat)",
    verticalAlign: "middle",
  },
});

/** Extensions for one C# editor; `reveal` shows a frame in the CPU Trace tab. */
export function traceHeat(ctx: EditorDocContext, reveal: (frame: string) => void): Extension {
  if (ctx.virtual || !CSHARP.test(ctx.path)) return [];
  return [
    heatField,
    gutterLaneMarkers.from(heatField, (h) => h.markers),
    codeLenses.from(heatField, (h) => h.lenses),
    EditorView.decorations.from(heatField, (h) => h.times),
    theme,
    ViewPlugin.define((view) => new HeatSync(view, ctx.path, reveal)),
  ];
}
