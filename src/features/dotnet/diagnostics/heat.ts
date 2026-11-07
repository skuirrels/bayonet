/**
 * The CPU trace shown in the editor: the methods of each source file with their
 * time, and where the trace has them, its lines, from the trace open in the CPU
 * Trace tab. Kept apart from the Diagnostics store so editors do not load it.
 */
import { create } from "zustand";
import type { LineHeat, MethodHeat } from "./trace";

export interface TraceHeatState {
  /** Settings: show the open trace's times in the editor. */
  enabled: boolean;
  /** The trace file the heat comes from. */
  tracePath: string | null;
  byFile: Readonly<Record<string, MethodHeat[]>>;
  linesByFile: Readonly<Record<string, LineHeat[]>>;
}

const KEY = "openidea.dotnet.diagnostics.traceHeat";

function loadEnabled(): boolean {
  try {
    return globalThis.localStorage?.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

export const useTraceHeat = create<TraceHeatState>(() => ({
  enabled: loadEnabled(),
  tracePath: null,
  byFile: {},
  linesByFile: {},
}));

export function setTraceHeat(
  tracePath: string | null,
  byFile: Record<string, MethodHeat[]>,
  linesByFile: Record<string, LineHeat[]> = {},
) {
  useTraceHeat.setState({ tracePath, byFile, linesByFile });
}

export function setTraceHeatEnabled(enabled: boolean) {
  useTraceHeat.setState({ enabled });
  try {
    globalThis.localStorage?.setItem(KEY, enabled ? "on" : "off");
  } catch {
    // Not persisted.
  }
}

const EMPTY: MethodHeat[] = [];
const NO_LINES: LineHeat[] = [];

/** The heat shown in the editor of `path`. */
export function shownHeat(path: string): MethodHeat[] {
  const s = useTraceHeat.getState();
  return s.enabled ? (s.byFile[path] ?? EMPTY) : EMPTY;
}

/** The line times shown in the editor of `path`. */
export function shownLineHeat(path: string): LineHeat[] {
  const s = useTraceHeat.getState();
  return s.enabled ? (s.linesByFile[path] ?? NO_LINES) : NO_LINES;
}
