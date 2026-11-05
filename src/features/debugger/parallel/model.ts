/**
 * Parallel Stacks (DBG-6): every thread's call stack merged into one graph.
 * Threads that share callers share boxes: each box is a run of frames the same
 * threads are in, its callees' boxes above it, so the entry points sit at the
 * bottom and the threads fan out towards where each one is stopped.
 */
import type { StackFrame } from "@/bindings";

export interface ThreadStack {
  threadId: number;
  threadName: string;
  /** Innermost first, as DAP reports them. */
  frames: readonly StackFrame[];
}

export interface BoxFrame {
  /** Method name as the adapter reports it. */
  name: string;
  path: string | null;
  /** 0-based line per thread (threads in one method may be on different lines). */
  lines: Map<number, number>;
  /** Index into each thread's stack, innermost being 0. */
  indexByThread: Map<number, number>;
}

export interface StackBox {
  id: string;
  /** Outermost (caller) first. */
  frames: BoxFrame[];
  threads: number[];
  /** Callee boxes, drawn above this one. */
  children: StackBox[];
}

/** Frames of the same method in the same file merge; the line may differ. */
const frameKey = (f: StackFrame) => `${f.name}\n${f.path ?? ""}`;

interface TrieNode {
  key: string;
  frame: BoxFrame;
  threads: number[];
  children: Map<string, TrieNode>;
}

/** The merged graph: one root box per distinct outermost frame. */
export function buildStacks(stacks: readonly ThreadStack[]): StackBox[] {
  const roots = new Map<string, TrieNode>();
  for (const s of stacks) {
    let level = roots;
    for (let i = s.frames.length - 1; i >= 0; i--) {
      const f = s.frames[i]!;
      const key = frameKey(f);
      let node = level.get(key);
      if (!node) {
        node = {
          key,
          frame: { name: f.name, path: f.path, lines: new Map(), indexByThread: new Map() },
          threads: [],
          children: new Map(),
        };
        level.set(key, node);
      }
      node.threads.push(s.threadId);
      node.frame.lines.set(s.threadId, f.line);
      node.frame.indexByThread.set(s.threadId, i);
      level = node.children;
    }
  }
  let next = 0;
  // A chain of frames the same threads pass through is one box.
  const toBox = (node: TrieNode): StackBox => {
    const frames = [node.frame];
    let at = node;
    while (at.children.size === 1) {
      const only = [...at.children.values()][0]!;
      if (only.threads.length !== at.threads.length) break;
      frames.push(only.frame);
      at = only;
    }
    return {
      id: `box${next++}`,
      frames,
      threads: [...node.threads],
      children: [...at.children.values()].map(toBox),
    };
  };
  return [...roots.values()].map(toBox);
}

export interface PlacedBox {
  box: StackBox;
  x: number;
  y: number;
  width: number;
  height: number;
  /** The box below (the caller), if any. */
  parent: string | null;
}

export interface StacksLayout {
  boxes: PlacedBox[];
  width: number;
  height: number;
}

export const HEADER_HEIGHT = 22;
export const ROW_HEIGHT = 20;
const H_GAP = 24;
const V_GAP = 36;
const MARGIN = 16;
const CHAR_WIDTH = 7;
const MIN_WIDTH = 140;
const MAX_WIDTH = 380;

const boxWidth = (b: StackBox) =>
  Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, ...b.frames.map((f) => f.name.length * CHAR_WIDTH + 24)));
const boxHeight = (b: StackBox) => HEADER_HEIGHT + b.frames.length * ROW_HEIGHT;

/** Callers at the bottom, callees above; each subtree centred over its own width. */
export function layoutStacks(roots: readonly StackBox[]): StacksLayout {
  const span = new Map<string, number>();
  const measure = (b: StackBox): number => {
    const kids = b.children.reduce((w, c) => w + measure(c), 0) + H_GAP * Math.max(0, b.children.length - 1);
    const w = Math.max(boxWidth(b), kids);
    span.set(b.id, w);
    return w;
  };
  roots.forEach(measure);
  const placed: PlacedBox[] = [];
  // `bottom` grows upwards from 0; flipped once the height is known.
  const place = (b: StackBox, left: number, bottom: number, parent: string | null) => {
    const total = span.get(b.id)!;
    const width = boxWidth(b);
    const height = boxHeight(b);
    placed.push({ box: b, x: left + (total - width) / 2, y: bottom + height, width, height, parent });
    const kids = b.children.reduce((w, c) => w + span.get(c.id)!, 0) + H_GAP * Math.max(0, b.children.length - 1);
    let x = left + (total - kids) / 2;
    for (const c of b.children) {
      place(c, x, bottom + height + V_GAP, b.id);
      x += span.get(c.id)! + H_GAP;
    }
  };
  let x = MARGIN;
  for (const r of roots) {
    place(r, x, MARGIN, null);
    x += span.get(r.id)! + H_GAP;
  }
  const top = Math.max(MARGIN, ...placed.map((p) => p.y)) + MARGIN;
  for (const p of placed) p.y = top - p.y;
  return {
    boxes: placed,
    width: Math.max(0, x - H_GAP) + MARGIN,
    height: top,
  };
}

/** `1 Thread` / `3 Threads` headers count; the tooltip names them. */
export function threadNames(box: StackBox, stacks: readonly ThreadStack[]): string[] {
  return box.threads.map((id) => stacks.find((s) => s.threadId === id)?.threadName ?? `#${id}`);
}
