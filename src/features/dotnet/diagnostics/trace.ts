/**
 * CPU trace model for the CPU Trace tab and the editor heat: per-method totals,
 * callers and callees of one method (the butterfly view), and the heat of the
 * methods the trace found in the solution's sources.
 */
import type { FlameNode, FrameSource, TraceProfile } from "@/bindings";

export interface MethodRow {
  /** The full frame name (`Module!Namespace.Type.Method(args)`). */
  name: string;
  selfMs: number;
  /** Inclusive time; a recursive method's nested calls count once. */
  totalMs: number;
  /** Call-tree nodes of this method (distinct call paths). */
  paths: number;
}

/** Every method of the trace by self time, heaviest first. */
export function methodTable(root: FlameNode): MethodRow[] {
  const by = new Map<string, MethodRow>();
  const walk = (n: FlameNode, onStack: Set<string>) => {
    let row = by.get(n.name);
    if (!row) {
      row = { name: n.name, selfMs: 0, totalMs: 0, paths: 0 };
      by.set(n.name, row);
    }
    row.selfMs += n.selfMs;
    row.paths++;
    const outer = !onStack.has(n.name);
    if (outer) {
      row.totalMs += n.totalMs;
      onStack.add(n.name);
    }
    for (const c of n.children) walk(c, onStack);
    if (outer) onStack.delete(n.name);
  };
  for (const c of root.children) walk(c, new Set());
  return [...by.values()].sort((a, b) => b.selfMs - a.selfMs || b.totalMs - a.totalMs || a.name.localeCompare(b.name));
}

export interface Neighbour {
  name: string;
  ms: number;
}

export interface Butterfly {
  method: string;
  selfMs: number;
  totalMs: number;
  /** Who called the method, by the method's time under each caller. */
  callers: Neighbour[];
  /** What the method called, by their time under it. */
  callees: Neighbour[];
}

const byMs = (m: Map<string, number>): Neighbour[] =>
  [...m.entries()].map(([name, ms]) => ({ name, ms })).sort((a, b) => b.ms - a.ms || a.name.localeCompare(b.name));

/**
 * Callers and callees of `method`. Only its outermost calls count (a recursive
 * method is not its own caller), so the callers add up to the method's total.
 */
export function butterfly(root: FlameNode, method: string): Butterfly {
  const callers = new Map<string, number>();
  const callees = new Map<string, number>();
  let selfMs = 0;
  let totalMs = 0;
  const walk = (n: FlameNode, parent: FlameNode | null, inside: boolean) => {
    const here = n.name === method;
    if (here) {
      selfMs += n.selfMs;
      if (!inside) {
        totalMs += n.totalMs;
        const caller = parent ? parent.name : ROOT_CALLER;
        callers.set(caller, (callers.get(caller) ?? 0) + n.totalMs);
      }
      for (const c of n.children) {
        if (c.name !== method) callees.set(c.name, (callees.get(c.name) ?? 0) + c.totalMs);
      }
    }
    for (const c of n.children) walk(c, n, inside || here);
  };
  for (const c of root.children) walk(c, null, false);
  return { method, selfMs, totalMs, callers: byMs(callers), callees: byMs(callees) };
}

/** Caller name of a method at the top of a thread's stack. */
export const ROOT_CALLER = "";

/**
 * Call-tree keys (`"0/2/1"`, child indexes from the root) to expand first: the path to the
 * frames where `isOwn` code spends most of its own time, or the hottest path when the
 * trace has none of it.
 */
export function initialCallPath(root: FlameNode, isOwn: (name: string) => boolean): Set<string> {
  const total = root.totalMs || 1;
  const own = new Map<FlameNode, number>();
  const ownMs = (n: FlameNode): number => {
    let ms = own.get(n);
    if (ms === undefined) {
      ms = (isOwn(n.name) ? n.selfMs : 0) + n.children.reduce((sum, c) => sum + ownMs(c), 0);
      own.set(n, ms);
    }
    return ms;
  };
  const weight = ownMs(root) / total >= 0.005 ? ownMs : (n: FlameNode) => n.totalMs;
  const keys = new Set<string>();
  let node = root;
  let key = "";
  for (let depth = 0; depth < 60; depth++) {
    let best = -1;
    node.children.forEach((c, i) => {
      if (best < 0 || weight(c) > weight(node.children[best]!)) best = i;
    });
    const next = node.children[best];
    if (!next || weight(next) / total < 0.005) break;
    if (key) keys.add(key);
    key = key ? `${key}/${best}` : String(best);
    node = next;
  }
  return keys;
}

/** A frame name split for display. */
export interface FrameDisplay {
  module: string;
  /** `Calculator.Spin`, `Worker.RunAsync`, `Calculator+Nested..ctor`. */
  method: string;
  /** The namespace, `""` when none. */
  namespace: string;
  /** The parenthesised arguments, `""` when none. */
  args: string;
}

const STATE_MACHINE = /\+<([^>]+)>d__\d+\.MoveNext$/;
const LAMBDA = /\+<>c(?:__DisplayClass[\d_]+)?\.<([^>]+)>b__[\d_]+$/;
const LOCAL = /\.<([^>]+)>g__([^|]+)\|[\d_]+$/;

/**
 * Shows a frame the way the code reads: `Worker+<RunAsync>d__0.MoveNext()` is
 * `Worker.RunAsync (async)`, lambdas and local functions are named after the
 * method that holds them, generic instantiations are dropped.
 */
export function displayFrame(name: string): FrameDisplay {
  const bang = name.indexOf("!");
  if (bang <= 0) return { module: "", method: name, namespace: "", args: "" };
  const module = name.slice(0, bang);
  let rest = name.slice(bang + 1);
  const open = rest.indexOf("(");
  const args = open >= 0 ? rest.slice(open) : "";
  if (open >= 0) rest = rest.slice(0, open);
  rest = rest.replace(/\[[^\]]*\]/g, "").replace(/`\d+/g, "");
  let suffix = "";
  let m: RegExpExecArray | null;
  if ((m = STATE_MACHINE.exec(rest))) {
    rest = `${rest.slice(0, m.index)}.${m[1]}`;
    suffix = " (async)";
  } else if ((m = LAMBDA.exec(rest))) {
    rest = `${rest.slice(0, m.index)}.${m[1]}`;
    suffix = " (lambda)";
  } else if ((m = LOCAL.exec(rest))) {
    rest = `${rest.slice(0, m.index)}.${m[2]}`;
    suffix = ` (local function in ${m[1]})`;
  }
  // `Ns.Type+Inner.Method`: the namespace is before the first type.
  const ctor = rest.endsWith("..ctor") ? "..ctor" : rest.endsWith("..cctor") ? "..cctor" : "";
  const body = ctor ? rest.slice(0, -ctor.length) : rest;
  const plus = body.indexOf("+");
  const head = plus >= 0 ? body.slice(0, plus) : body;
  const dots = head.split(".");
  // Without `+` or `.ctor` the head ends with the method; otherwise with the outer type.
  const typeStart = Math.max(0, plus >= 0 || ctor ? dots.length - 1 : dots.length - 2);
  const namespace = dots.slice(0, typeStart).join(".");
  const method = `${dots.slice(typeStart).join(".")}${plus >= 0 ? body.slice(plus) : ""}${ctor}`.replace(/\+/g, ".");
  return { module, method: method + suffix, namespace, args };
}

// ---------------------------------------------------------------- editor heat

/** Share of thread time (0–1) → heat level 1 (warm) to 5 (hottest); below 0.5 % a method is not marked (0). */
export function heatLevel(share: number): number {
  if (!(share >= 0.005)) return 0;
  if (share >= 0.25) return 5;
  if (share >= 0.1) return 4;
  if (share >= 0.04) return 3;
  if (share >= 0.015) return 2;
  return 1;
}

export interface MethodHeat {
  frame: string;
  /** 0-based line and column of the method's name. */
  line: number;
  col: number;
  /** 0-based first and last line of the declaration. */
  startLine: number;
  endLine: number;
  selfMs: number;
  totalMs: number;
  /** Shares of all thread time, 0–1. */
  totalShare: number;
  selfShare: number;
  level: number;
}

/**
 * The methods each source file declares, with their time. Frames that resolve to
 * the same declaration (an async method and its state machine, a method and its
 * lambdas) are added together.
 */
export function heatByFile(trace: TraceProfile): Record<string, MethodHeat[]> {
  const total = trace.root.totalMs || 1;
  const rows = new Map(methodTable(trace.root).map((r) => [r.name, r]));
  const byDecl = new Map<string, { source: FrameSource; selfMs: number; totalMs: number }>();
  for (const s of trace.sources) {
    const row = rows.get(s.frame);
    if (!row) continue;
    const key = `${s.path}\u0000${s.line}\u0000${s.col}`;
    const e = byDecl.get(key);
    // Inclusive times of frames of one declaration nest (MoveNext under the
    // async method's stub), so the larger is the declaration's total.
    if (e) {
      e.selfMs += row.selfMs;
      e.totalMs = Math.max(e.totalMs, row.totalMs);
    } else byDecl.set(key, { source: s, selfMs: row.selfMs, totalMs: row.totalMs });
  }
  const out: Record<string, MethodHeat[]> = {};
  for (const { source: s, selfMs, totalMs } of byDecl.values()) {
    const totalShare = totalMs / total;
    (out[s.path] ??= []).push({
      frame: s.frame,
      line: s.line,
      col: s.col,
      startLine: s.startLine,
      endLine: s.endLine,
      selfMs,
      totalMs,
      totalShare,
      selfShare: selfMs / total,
      level: heatLevel(totalShare),
    });
  }
  for (const list of Object.values(out)) list.sort((a, b) => a.line - b.line);
  return out;
}

export interface LineHeat {
  /** 0-based. */
  line: number;
  selfMs: number;
  totalMs: number;
  /** Shares of all thread time, 0–1. */
  totalShare: number;
  selfShare: number;
  level: number;
}

/**
 * The source lines of each file with their time, where the trace has lines (one the
 * .NET diagnostics helper read), hottest first within a file. Lines under the
 * heat threshold are left out.
 */
export function lineHeatByFile(trace: TraceProfile): Record<string, LineHeat[]> {
  const total = trace.root.totalMs || 1;
  const out: Record<string, LineHeat[]> = {};
  for (const l of trace.lines) {
    // Floats arrive as `number | null` (a NaN would be null).
    const totalMs = l.totalMs ?? 0;
    const selfMs = l.selfMs ?? 0;
    const totalShare = totalMs / total;
    const level = heatLevel(totalShare);
    if (level === 0) continue;
    (out[l.path] ??= []).push({ line: l.line, selfMs, totalMs, totalShare, selfShare: selfMs / total, level });
  }
  for (const list of Object.values(out)) list.sort((a, b) => a.line - b.line);
  return out;
}

export interface GcSummary {
  count: number;
  /** Collections per generation (0, 1, 2). */
  byGeneration: [number, number, number];
  pauseMs: number;
  /** Pause time as a share of the trace's duration, 0–1. */
  pauseShare: number;
  longestPauseMs: number;
}

export function gcSummary(trace: TraceProfile): GcSummary | null {
  const gcs = trace.collections;
  if (!gcs.length) return null;
  const byGeneration: [number, number, number] = [0, 0, 0];
  let pauseMs = 0;
  let longestPauseMs = 0;
  for (const c of gcs) {
    byGeneration[Math.min(c.generation, 2) as 0 | 1 | 2]++;
    pauseMs += c.pauseMs ?? 0;
    longestPauseMs = Math.max(longestPauseMs, c.pauseMs ?? 0);
  }
  return { count: gcs.length, byGeneration, pauseMs, pauseShare: pauseMs / (trace.durationMs || 1), longestPauseMs };
}

// ---------------------------------------------------------------- formatting

/** `1.24 s`, `86.3 ms`, `4 ms`: three significant figures. */
export function formatMs(ms: number): string {
  if (ms >= 1000) {
    const s = ms / 1000;
    return `${s >= 100 ? s.toFixed(0) : s >= 10 ? s.toFixed(1) : s.toFixed(2)} s`;
  }
  return `${ms >= 100 || Number.isInteger(ms) ? ms.toFixed(0) : ms.toFixed(1)} ms`;
}

/** `18.2 %`, `0.4 %`, `< 0.1 %`. */
export function formatShare(share: number): string {
  const pct = share * 100;
  if (pct > 0 && pct < 0.1) return "< 0.1 %";
  return `${pct >= 10 ? pct.toFixed(0) : pct.toFixed(1)} %`;
}
