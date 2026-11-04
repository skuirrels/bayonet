/**
 * The CPU trace shown in the editor: the methods of each source file with their
 * time, from the trace open in the CPU Trace tab. Kept apart from the Diagnostics
 * store so editors do not load it.
 */
import { create } from "zustand";
import type { MethodHeat } from "./trace";

export interface TraceHeatState {
  /** Settings: show the open trace's times in the editor. */
  enabled: boolean;
  /** The trace file the heat comes from. */
  tracePath: string | null;
  byFile: Readonly<Record<string, MethodHeat[]>>;
}

const KEY = "openidea.dotnet.diagnostics.traceHeat";

function loadEnabled(): boolean {
  try {
    return globalThis.localStorage?.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

export const useTraceHeat = create<TraceHeatState>(() => ({ enabled: loadEnabled(), tracePath: null, byFile: {} }));

export function setTraceHeat(tracePath: string | null, byFile: Record<string, MethodHeat[]>) {
  useTraceHeat.setState({ tracePath, byFile });
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

/** The heat shown in the editor of `path`. */
export function shownHeat(path: string): MethodHeat[] {
  const s = useTraceHeat.getState();
  return s.enabled ? (s.byFile[path] ?? EMPTY) : EMPTY;
}
