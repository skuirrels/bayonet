/**
 * Target framework switcher: a file of a multi-targeting project is compiled once
 * per framework, and Roslyn offers each as a project context. The chosen one decides
 * which `#if` branches are live, what completes and what is reported.
 */
import { create } from "zustand";
import type { ProjectContext, ProjectContexts } from "@/bindings";
import { call, commands, isTauri } from "@/platform/ipc";

/** Languages Roslyn serves, by backend language id. */
export const ROSLYN_LANGUAGES = new Set(["csharp", "vb"]);

/** `Fixture.Core (net10.0)` → `net10.0`; other labels as they are. */
export function frameworkOf(label: string): string {
  return /\(([^()]+)\)\s*$/.exec(label)?.[1] ?? label;
}

/** Only real choices: one context (or only "Miscellaneous Files") needs no switcher. */
export function hasChoice(contexts: ProjectContexts | null): contexts is ProjectContexts {
  return !!contexts && contexts.contexts.length > 1;
}

interface FrameworkState {
  /** The active editor's document and its contexts. */
  doc: number | null;
  path: string | null;
  contexts: ProjectContexts | null;
  /** Chosen context id per file, re-applied when the file opens again. */
  chosen: ReadonlyMap<string, string>;
}

export const useFrameworks = create<FrameworkState>(() => ({
  doc: null,
  path: null,
  contexts: null,
  chosen: new Map(),
}));

let seq = 0;

/** Loads the contexts of `doc` (a C# or VB file), re-applying the file's earlier choice. */
export async function loadContexts(doc: number | null, path: string | null): Promise<void> {
  const id = ++seq;
  useFrameworks.setState({ doc, path, contexts: null });
  if (doc === null || doc < 0 || !path || !isTauri()) return;
  let contexts: ProjectContexts | null;
  try {
    contexts = await call(commands.lspProjectContexts(doc));
  } catch {
    contexts = null;
  }
  if (id !== seq) return;
  const want = useFrameworks.getState().chosen.get(path);
  const index = want ? (contexts?.contexts.findIndex((c) => c.id === want) ?? -1) : -1;
  if (contexts && index >= 0 && index !== contexts.active) {
    await call(commands.lspSetProjectContext(doc, contexts.contexts[index]!)).catch(() => {});
    if (id !== seq) return;
    contexts = { ...contexts, active: index };
  }
  useFrameworks.setState({ contexts });
}

/** Makes the active document use `context`. */
export async function chooseContext(context: ProjectContext): Promise<void> {
  const { doc, path, contexts, chosen } = useFrameworks.getState();
  if (doc === null || !path || !contexts) return;
  const index = contexts.contexts.findIndex((c) => c.id === context.id);
  if (index < 0) return;
  await call(commands.lspSetProjectContext(doc, context));
  useFrameworks.setState({
    contexts: { ...contexts, active: index },
    chosen: new Map(chosen).set(path, context.id),
  });
}
