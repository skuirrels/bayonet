/**
 * The .NET Diagnostics tool window's data (DIAG): live counter series, grouped
 * thread stacks, the flame graph layout, heap snapshots and their differences, and
 * the trigger rules evaluated on the live counters. Pure, so the view and the
 * tests share it.
 */
import type { FlameNode, HeapType, ManagedFrame, MetricSample, ThreadStack } from "@/bindings";

// ---------------------------------------------------------------- live counters

/** Points kept per counter: five minutes at one per second. */
export const MAX_POINTS = 300;

export interface Point {
  /** Milliseconds since the epoch. */
  t: number;
  v: number;
}

export interface Series {
  key: string;
  name: string;
  displayName: string;
  provider: string;
  unit: string;
  counterType: string;
  tags: string;
  points: Point[];
}

export const seriesKey = (s: Pick<MetricSample, "provider" | "name" | "tags">) =>
  s.tags ? `${s.provider}/${s.name}[${s.tags}]` : `${s.provider}/${s.name}`;

/** Adds a batch of samples to the series (a new map; untouched series are kept as they are). */
export function addSamples(
  series: Readonly<Record<string, Series>>,
  batch: readonly MetricSample[],
  max = MAX_POINTS,
): Record<string, Series> {
  const out: Record<string, Series> = { ...series };
  for (const s of batch) {
    if (s.time == null || s.value == null || !Number.isFinite(s.value)) continue;
    const key = seriesKey(s);
    const prev = out[key];
    const points = prev ? [...prev.points, { t: s.time, v: s.value }] : [{ t: s.time, v: s.value }];
    if (points.length > max) points.splice(0, points.length - max);
    out[key] = {
      key,
      name: s.name,
      displayName: s.tags ? `${s.displayName} (${s.tags})` : s.displayName,
      provider: s.provider,
      unit: s.unit,
      counterType: s.counterType,
      tags: s.tags,
      points,
    };
  }
  return out;
}

/** The counters the Live tab shows as tiles, in order (System.Runtime event counter names). */
export const HEADLINE_COUNTERS = [
  "cpu-usage",
  "working-set",
  "gc-heap-size",
  "threadpool-thread-count",
  "threadpool-queue-length",
  "exception-count",
] as const;

/** Headline series first (in their order), then the rest by provider and name. */
export function orderSeries(series: Readonly<Record<string, Series>>): Series[] {
  const rank = (s: Series) => {
    const i = (HEADLINE_COUNTERS as readonly string[]).indexOf(s.name);
    return i < 0 || s.tags ? HEADLINE_COUNTERS.length : i;
  };
  return Object.values(series).sort(
    (a, b) => rank(a) - rank(b) || a.provider.localeCompare(b.provider) || a.displayName.localeCompare(b.displayName),
  );
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"];

/** `1,536` B → `1.5 KB`. */
export function formatBytes(bytes: number): string {
  let v = Math.abs(bytes);
  let i = 0;
  while (v >= 1024 && i < BYTE_UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const sign = bytes < 0 ? "-" : "";
  return `${sign}${i === 0 ? v.toFixed(0) : v.toFixed(v >= 100 ? 0 : 1)} ${BYTE_UNITS[i]}`;
}

/** A counter value with its unit, compact (`12.5 %`, `68 MB`, `1.2k`). */
export function formatValue(v: number, unit: string): string {
  const u = unit.trim();
  if (u === "B" || u === "By" || u === "bytes") return formatBytes(v);
  if (u === "MB") return v >= 1024 ? `${(v / 1024).toFixed(2)} GB` : `${v.toFixed(v >= 100 ? 0 : 1)} MB`;
  const abs = Math.abs(v);
  const n =
    abs >= 1e9
      ? `${(v / 1e9).toFixed(1)}G`
      : abs >= 1e6
        ? `${(v / 1e6).toFixed(1)}M`
        : abs >= 1e4
          ? `${(v / 1e3).toFixed(1)}k`
          : Number.isInteger(v)
            ? String(v)
            : v.toFixed(abs >= 100 ? 0 : abs >= 1 ? 1 : 2);
  if (!u || u === "count") return n;
  return u === "%" ? `${n} %` : `${n} ${u}`;
}

/** A round axis maximum at or above `v` (1, 2, 2.5, 5 × 10ⁿ). */
export function niceMax(v: number): number {
  if (!(v > 0)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (v <= m * p) return m * p;
  return 10 * p;
}

/** SVG path of a line through `points` in a `width` × `height` box spanning [t0, t1] × [0, yMax]. */
export function linePath(
  points: readonly Point[],
  width: number,
  height: number,
  t0: number,
  t1: number,
  yMax: number,
) {
  if (!points.length) return "";
  const span = Math.max(1, t1 - t0);
  const x = (t: number) => ((t - t0) / span) * width;
  const y = (v: number) => height - (Math.max(0, Math.min(v, yMax)) / (yMax || 1)) * height;
  return points.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join("");
}

/** The point nearest to `t`. */
export function nearestPoint(points: readonly Point[], t: number): Point | undefined {
  let best: Point | undefined;
  for (const p of points) if (!best || Math.abs(p.t - t) < Math.abs(best.t - t)) best = p;
  return best;
}

// ---------------------------------------------------------------- stacks

export interface StackGroup {
  key: string;
  /** Innermost first. */
  frames: ManagedFrame[];
  threads: string[];
}

/** Threads with identical stacks, most threads first. */
export function groupStacks(threads: readonly ThreadStack[]): StackGroup[] {
  const groups = new Map<string, StackGroup>();
  for (const t of threads) {
    const key = t.frames.map((f) => `${f.module}!${f.method}`).join("\n");
    const g = groups.get(key);
    if (g) g.threads.push(t.id);
    else groups.set(key, { key, frames: t.frames, threads: [t.id] });
  }
  return [...groups.values()].sort((a, b) => b.threads.length - a.threads.length || b.frames.length - a.frames.length);
}

/** The frame a stack group is named after: the innermost one of `userModules`, else the innermost managed one. */
export function headlineFrame(
  frames: readonly ManagedFrame[],
  userModules: ReadonlySet<string>,
): ManagedFrame | undefined {
  return frames.find((f) => userModules.has(f.module)) ?? frames.find((f) => f.module) ?? frames[0];
}

/** `System.Threading.Monitor.Wait(class System.Object,int32)` → `Monitor.Wait`. */
export function shortMethod(method: string): string {
  const name = method.split("(")[0] ?? method;
  const parts = name.split(".");
  return parts.length > 2 ? parts.slice(-2).join(".") : name;
}

// ---------------------------------------------------------------- flame graph

export type FrameCategory = "user" | "dotnet" | "native";

/** Speedscope frame names are `Module!Method`; others are native or runtime frames. */
export function frameCategory(name: string, userModules: ReadonlySet<string>): FrameCategory {
  const bang = name.indexOf("!");
  if (bang <= 0) return "native";
  const module = name.slice(0, bang);
  if (module === "?" || module === "UNMANAGED_CODE_TIME") return "native";
  return userModules.has(module) ? "user" : "dotnet";
}

export interface FlameCell {
  node: FlameNode;
  /** Child indexes from the root, `""` for the root. */
  path: string;
  depth: number;
  /** Left edge and width as fractions of the view. */
  x: number;
  w: number;
  /** An ancestor of the zoomed node (drawn full width). */
  ancestor: boolean;
}

/** The node at `path` (child indexes joined by `/`). */
export function nodeAt(root: FlameNode, path: string): FlameNode | undefined {
  let n: FlameNode | undefined = root;
  for (const i of path ? path.split("/") : []) n = n?.children[Number(i)];
  return n;
}

/**
 * Lays out an icicle (callers on top): the zoomed node spans the width with its
 * ancestors above it; cells narrower than `minWidth` are left out.
 */
export function layoutFlame(root: FlameNode, zoom = "", minWidth = 0.002): FlameCell[] {
  const cells: FlameCell[] = [];
  const parts = zoom ? zoom.split("/") : [];
  let node = root;
  let path = "";
  for (let d = 0; d < parts.length; d++) {
    const next = node.children[Number(parts[d])];
    if (!next) break;
    cells.push({ node, path, depth: d, x: 0, w: 1, ancestor: true });
    node = next;
    path = path ? `${path}/${parts[d]}` : parts[d]!;
  }
  const base = cells.length;
  const total = node.totalMs || 1;
  const walk = (n: FlameNode, p: string, depth: number, x: number) => {
    const w = n.totalMs / total;
    if (w < minWidth) return;
    cells.push({ node: n, path: p, depth, x, w, ancestor: false });
    let cx = x;
    n.children.forEach((c, i) => {
      walk(c, p ? `${p}/${i}` : String(i), depth + 1, cx);
      cx += c.totalMs / total;
    });
  };
  walk(node, path, base, 0);
  return cells;
}

/** The heaviest frames by self time across the tree (the "hot spots" list). */
export function hotFrames(root: FlameNode, count = 20): { name: string; selfMs: number; totalMs: number }[] {
  const by = new Map<string, { name: string; selfMs: number; totalMs: number }>();
  const walk = (n: FlameNode, onStack: Set<string>) => {
    const e = by.get(n.name) ?? { name: n.name, selfMs: 0, totalMs: 0 };
    e.selfMs += n.selfMs;
    // Recursion counts a frame's inclusive time once.
    if (!onStack.has(n.name)) e.totalMs += n.totalMs;
    by.set(n.name, e);
    onStack.add(n.name);
    for (const c of n.children) walk(c, onStack);
    onStack.delete(n.name);
  };
  for (const c of root.children) walk(c, new Set());
  return [...by.values()].sort((a, b) => b.selfMs - a.selfMs).slice(0, count);
}

// ---------------------------------------------------------------- heap

export interface HeapRow extends HeapType {
  /** Change since the baseline snapshot, when comparing. */
  deltaBytes?: number;
  deltaCount?: number;
}

const typeKey = (t: Pick<HeapType, "name" | "module">) => `${t.module}|${t.name}`;

/** `after`'s types with their growth since `before`, biggest growth first; types gone since are kept at zero. */
export function heapDiff(before: readonly HeapType[], after: readonly HeapType[]): HeapRow[] {
  const old = new Map(before.map((t) => [typeKey(t), t]));
  const rows: HeapRow[] = after.map((t) => {
    const b = old.get(typeKey(t));
    old.delete(typeKey(t));
    return { ...t, deltaBytes: t.bytes - (b?.bytes ?? 0), deltaCount: t.count - (b?.count ?? 0) };
  });
  for (const b of old.values()) rows.push({ ...b, bytes: 0, count: 0, deltaBytes: -b.bytes, deltaCount: -b.count });
  return rows.sort((a, b) => (b.deltaBytes ?? 0) - (a.deltaBytes ?? 0) || b.bytes - a.bytes);
}

export function filterHeap<T extends HeapType>(rows: readonly T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  return q ? rows.filter((r) => r.name.toLowerCase().includes(q) || r.module.toLowerCase().includes(q)) : [...rows];
}

// ---------------------------------------------------------------- triggers

export type TriggerAction = "gcdump" | "dump" | "trace" | "stacks";

export interface TriggerRule {
  id: string;
  enabled: boolean;
  /** Series name (`cpu-usage`). */
  counter: string;
  op: ">" | "<";
  threshold: number;
  /** The condition must hold for this long. */
  seconds: number;
  action: TriggerAction;
  /** No second firing for this long. */
  cooldownSeconds: number;
}

export const DEFAULT_RULES: TriggerRule[] = [
  {
    id: "high-cpu",
    enabled: false,
    counter: "cpu-usage",
    op: ">",
    threshold: 80,
    seconds: 10,
    action: "trace",
    cooldownSeconds: 300,
  },
  {
    id: "big-heap",
    enabled: false,
    counter: "gc-heap-size",
    op: ">",
    threshold: 512,
    seconds: 5,
    action: "gcdump",
    cooldownSeconds: 600,
  },
];

/**
 * Whether `rule` holds at `now`: the counter has values covering the last
 * `seconds` (allowing one missed sample), all of them past the threshold, and
 * the rule is out of its cool-down.
 */
export function ruleFires(
  rule: TriggerRule,
  series: Readonly<Record<string, Series>>,
  now: number,
  lastFired?: number,
): boolean {
  if (!rule.enabled) return false;
  if (lastFired !== undefined && now - lastFired < rule.cooldownSeconds * 1000) return false;
  const s = Object.values(series).find((x) => x.name === rule.counter && !x.tags);
  if (!s?.points.length) return false;
  const from = now - rule.seconds * 1000;
  const window = s.points.filter((p) => p.t >= from);
  if (!window.length || window[0]!.t > from + 1500) return false;
  return window.every((p) => (rule.op === ">" ? p.v > rule.threshold : p.v < rule.threshold));
}

// ---------------------------------------------------------------- collected files

/** `/…/diagnostics/57544-1762245926123.gcdump` → its pid and when it was collected. */
export function artifactInfo(path: string): { pid: number | null; time: number | null } {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const m = /^(\d+)-(\d{10,})\./.exec(name);
  return m ? { pid: Number(m[1]), time: Number(m[2]) } : { pid: null, time: null };
}

// ---------------------------------------------------------------- processes

export type ProcessGroup = "project" | "apps" | "tooling";

/** Build servers, language servers and IDEs: rarely what is being diagnosed. */
const TOOLING =
  /(?:^|[/\\])(?:MSBuild(?:\.dll)?|VBCSCompiler(?:\.dll)?|Rider\.Backend|Microsoft\.CodeAnalysis\.LanguageServer|roslyn-language-server|csharp-ls|fsautocomplete|dotnet-(?:monitor|counters|trace|dump|gcdump|stack|debug|ef))\b/i;

/** Started from the project's folder, an app elsewhere, or tooling. */
export function processGroup(p: { name: string; commandLine: string }, root: string | null): ProcessGroup {
  if (
    TOOLING.test(p.name) ||
    TOOLING.test(p.commandLine.split(" ")[1] ?? "") ||
    TOOLING.test(p.commandLine.split(" ")[0] ?? "")
  )
    return "tooling";
  return root && p.commandLine.includes(root) ? "project" : "apps";
}

export const PROCESS_GROUPS: ProcessGroup[] = ["project", "apps", "tooling"];
