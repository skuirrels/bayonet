/**
 * Charts of the Diagnostics window: a counter's line over time (crosshair and
 * read-out on hover), sparklines for the counter tiles, and the CPU flame graph
 * (an icicle: callers on top, width = time; click to zoom).
 */
import { useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import clsx from "clsx";
import type { FlameNode } from "@/bindings";
import { tx } from "@/platform/i18n";
import {
  formatValue,
  frameCategory,
  layoutFlame,
  linePath,
  nearestPoint,
  niceMax,
  type FlameCell,
  type FrameCategory,
  type Point,
  type Series,
} from "./model";
import styles from "./Diagnostics.module.css";

const clock = (t: number) =>
  new Date(t).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

const extent = (points: readonly Point[]) => ({
  t0: points[0]?.t ?? 0,
  t1: points.at(-1)?.t ?? 1,
  max: niceMax(Math.max(0, ...points.map((p) => p.v))),
});

/** A small line of a counter's recent values (the tile's trend; the tile shows the value). */
export function Sparkline({ points, className }: { points: readonly Point[]; className?: string }) {
  const { t0, t1, max } = extent(points);
  const d = linePath(points, 100, 24, t0, t1, max);
  return (
    <svg className={clsx(styles.sparkline, className)} viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden>
      <path d={`${d}L100,24L0,24Z`} className={styles.area} />
      <path d={d} className={styles.line} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

const CHART_H = 160;

/** A counter over time, with a hairline grid, the axis maximum and a crosshair read-out. */
export function LineChart({ series }: { series: Series }) {
  const { points, unit } = series;
  const counter = series.displayName;
  const { t0, t1, max } = extent(points);
  const [hover, setHover] = useState<Point | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const d = linePath(points, 1000, CHART_H, t0, t1, max);
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r || !points.length) return;
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    setHover(nearestPoint(points, t0 + f * (t1 - t0)) ?? null);
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const i = hover ? points.indexOf(hover) : points.length;
    const next = points[Math.min(points.length - 1, Math.max(0, i + (e.key === "ArrowLeft" ? -1 : 1)))];
    setHover(next ?? null);
  };
  const x = (t: number) => ((t - t0) / Math.max(1, t1 - t0)) * 100;
  const y = (v: number) => (1 - Math.min(v, max) / max) * 100;
  return (
    <figure className={styles.chart}>
      <div className={styles.chartAxis}>
        <span>{formatValue(max, unit)}</span>
        <span>{formatValue(max / 2, unit)}</span>
        <span>0</span>
      </div>
      <div
        ref={ref}
        className={styles.plot}
        style={{ height: CHART_H }}
        tabIndex={0}
        role="img"
        aria-label={tx`${counter} over time`}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
        onKeyDown={onKey}
        onBlur={() => setHover(null)}
      >
        <svg viewBox={`0 0 1000 ${CHART_H}`} preserveAspectRatio="none" className={styles.plotSvg} aria-hidden>
          {[0, 0.5, 1].map((f) => (
            <line
              key={f}
              x1={0}
              x2={1000}
              y1={f * CHART_H}
              y2={f * CHART_H}
              className={styles.grid}
              vectorEffect="non-scaling-stroke"
            />
          ))}
          <path d={`${d}L1000,${CHART_H}L0,${CHART_H}Z`} className={styles.area} />
          <path d={d} className={styles.line} vectorEffect="non-scaling-stroke" />
        </svg>
        {hover && (
          <>
            <span className={styles.crosshair} style={{ left: `${x(hover.t)}%` }} />
            <span className={styles.dot} style={{ left: `${x(hover.t)}%`, top: `${y(hover.v)}%` }} />
            <span
              className={styles.tooltip}
              style={x(hover.t) > 60 ? { right: `${100 - x(hover.t)}%` } : { left: `${x(hover.t)}%` }}
            >
              <strong>{formatValue(hover.v, unit)}</strong>
              <span>{clock(hover.t)}</span>
            </span>
          </>
        )}
      </div>
      <figcaption className={styles.chartTime}>
        <span>{points.length ? clock(t0) : ""}</span>
        <span>{points.length ? clock(t1) : ""}</span>
      </figcaption>
    </figure>
  );
}

// ---------------------------------------------------------------- flame graph

const ROW = 20;

const CATEGORY_LABEL: Record<FrameCategory, () => string> = {
  user: () => tx`Your code`,
  dotnet: () => tx`.NET libraries`,
  native: () => tx`Native and runtime`,
};

export function FlameLegend() {
  return (
    <div className={styles.legend}>
      {(["user", "dotnet", "native"] as const).map((c) => (
        <span key={c} className={styles.legendItem}>
          <span className={clsx(styles.swatch, styles[`cat-${c}`])} />
          {CATEGORY_LABEL[c]()}
        </span>
      ))}
    </div>
  );
}

export interface FlameGraphProps {
  root: FlameNode;
  zoom: string;
  onZoom: (path: string) => void;
  userModules: ReadonlySet<string>;
  /** Highlights frames whose name contains it. */
  query: string;
}

export function FlameGraph({ root, zoom, onZoom, userModules, query }: FlameGraphProps) {
  const cells = useMemo(() => layoutFlame(root, zoom), [root, zoom]);
  const [hover, setHover] = useState<FlameCell | null>(null);
  const depth = cells.reduce((m, c) => Math.max(m, c.depth), 0) + 1;
  const q = query.trim().toLowerCase();
  const total = root.totalMs || 1;
  return (
    <div className={styles.flameHost}>
      <div className={styles.flame} style={{ height: depth * ROW }} role="tree" aria-label={tx`CPU flame graph`}>
        {cells.map((c) => {
          const category = c.path === "" ? "native" : frameCategory(c.node.name, userModules);
          const dim = q && !c.node.name.toLowerCase().includes(q);
          return (
            <button
              key={`${c.path}|${c.ancestor}`}
              type="button"
              role="treeitem"
              className={clsx(
                styles.cell,
                styles[`cat-${category}`],
                (c.ancestor || c.path === "") && styles.cellAncestor,
                dim && styles.cellDim,
              )}
              style={{ left: `${c.x * 100}%`, width: `${c.w * 100}%`, top: c.depth * ROW }}
              aria-label={`${c.node.name}, ${c.node.totalMs} ms`}
              onClick={() => onZoom(c.path === zoom && c.path ? c.path.split("/").slice(0, -1).join("/") : c.path)}
              onPointerEnter={() => setHover(c)}
              onPointerLeave={() => setHover((h) => (h === c ? null : h))}
              onFocus={() => setHover(c)}
            >
              {c.w > 0.02 ? c.node.name : ""}
            </button>
          );
        })}
      </div>
      <div className={styles.flameStatus} aria-live="polite">
        {hover ? (
          <>
            <strong>{hover.node.totalMs.toLocaleString()} ms</strong>
            <span>
              {((hover.node.totalMs / total) * 100).toFixed(1)} % · {tx`self`} {hover.node.selfMs.toLocaleString()} ms
            </span>
            <span className={styles.flameName}>{hover.node.name}</span>
          </>
        ) : (
          <span className={styles.muted}>{tx`Click a frame to zoom into it; click it again to zoom out.`}</span>
        )}
      </div>
    </div>
  );
}
