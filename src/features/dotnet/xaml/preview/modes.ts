/** Avalonia XAML editor layout per file: Editor only / Editor and Preview / Preview only. */
import { create } from "zustand";

export type XamlMode = "editor" | "split" | "preview";

const DEFAULT_KEY = "openidea.xamlPreview.defaultMode";
const MODES: readonly XamlMode[] = ["editor", "split", "preview"];

function storedDefault(): XamlMode {
  try {
    const v = localStorage.getItem(DEFAULT_KEY);
    return MODES.includes(v as XamlMode) ? (v as XamlMode) : "split";
  } catch {
    return "split";
  }
}

interface ModesState {
  byPath: Record<string, XamlMode>;
  /** Mode of files not switched yet: the last one chosen. */
  fallback: XamlMode;
}

export const useXamlModes = create<ModesState>(() => ({ byPath: {}, fallback: storedDefault() }));

export const modeOf = (s: ModesState, path: string): XamlMode => s.byPath[path] ?? s.fallback;

export function setXamlMode(path: string, mode: XamlMode) {
  useXamlModes.setState((s) => ({ byPath: { ...s.byPath, [path]: mode }, fallback: mode }));
  try {
    localStorage.setItem(DEFAULT_KEY, mode);
  } catch {
    // Per-viewer convenience only.
  }
}
