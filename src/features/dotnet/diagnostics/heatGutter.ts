/**
 * CPU trace heat in C# editors: a lens above each sampled method (`18 % · 412 ms
 * self`) and a bar in the change lane over its lines, coloured from warm to hot by
 * its share of all thread time. Clicking a lens shows the method in the CPU Trace
 * tab. Positions follow edits until the next trace.
 */
import { RangeSet, StateEffect, StateField, type EditorState, type Extension } from "@codemirror/state";
import { GutterMarker, ViewPlugin, type EditorView } from "@codemirror/view";
import { LaneMarker, codeLenses, gutterLaneMarkers, type CodeLens, type EditorDocContext } from "@/editor/api";
import { tx } from "@/platform/i18n";
import { shownHeat, useTraceHeat } from "./heat";
import { displayFrame, formatMs, formatShare, type MethodHeat } from "./trace";

const CSHARP = /\.(cs|csx)$/i;

interface Heat {
  markers: RangeSet<GutterMarker>;
  lenses: readonly CodeLens[];
}

const NONE: Heat = { markers: RangeSet.empty, lenses: [] };

export const heatColor = (level: number) => `var(--perf-heat-${level})`;

function tooltip(m: MethodHeat): string {
  const name = displayFrame(m.frame).method;
  const total = formatMs(m.totalMs);
  const self = formatMs(m.selfMs);
  const share = formatShare(m.totalShare);
  return tx`${name}: ${total} in total (${share} of thread time), ${self} in the method itself. Click to show it in the CPU trace.`;
}

/** Lenses and lane markers for `heat` in `state`'s document. Exported for tests. */
export function buildHeat(state: EditorState, heat: readonly MethodHeat[], reveal: (frame: string) => void): Heat {
  if (!heat.length) return NONE;
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
    const marker = new LaneMarker(heatColor(m.level), "bar", title);
    const last = Math.min(m.endLine, doc.lines - 1);
    for (let l = Math.max(m.startLine, 0); l <= last; l++) markers.push(marker.range(doc.line(l + 1).from));
  }
  return { markers: RangeSet.of(markers, true), lenses };
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
    };
  },
});

class HeatSync {
  private readonly unsub: () => void;
  private shown: readonly MethodHeat[] | null = null;
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
    if (heat === this.shown) return;
    this.shown = heat;
    this.view.dispatch({ effects: setHeat.of(buildHeat(this.view.state, heat, this.reveal)) });
  }

  destroy() {
    this.destroyed = true;
    this.unsub();
  }
}

/** Extensions for one C# editor; `reveal` shows a frame in the CPU Trace tab. */
export function traceHeat(ctx: EditorDocContext, reveal: (frame: string) => void): Extension {
  if (ctx.virtual || !CSHARP.test(ctx.path)) return [];
  return [
    heatField,
    gutterLaneMarkers.from(heatField, (h) => h.markers),
    codeLenses.from(heatField, (h) => h.lenses),
    ViewPlugin.define((view) => new HeatSync(view, ctx.path, reveal)),
  ];
}
