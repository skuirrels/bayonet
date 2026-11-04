/**
 * Pure helpers of the Build tool window: console lines from streamed output,
 * the build → file → message tree, and IDEA's finish summaries.
 */
import type { BuildMessage, BuildResult, Severity } from "@/platform/ipc";

export type BuildStatus = "running" | "success" | "failed" | "cancelled";

export interface ConsoleLine {
  text: string;
  kind: "out" | "err" | "system";
}

/** Oldest lines are dropped beyond this (IDEA's console cycle buffer). */
export const MAX_CONSOLE_LINES = 20_000;

/** Appends streamed text (one or more `\n`-terminated lines) to `lines`. */
export function appendOutput(lines: ConsoleLine[], text: string, kind: ConsoleLine["kind"]): ConsoleLine[] {
  const parts = text.split("\n");
  if (parts.at(-1) === "") parts.pop();
  const next = lines.concat(parts.map((t) => ({ text: t.replace(/\r$/, ""), kind })));
  return next.length > MAX_CONSOLE_LINES ? next.slice(next.length - MAX_CONSOLE_LINES) : next;
}

/** A project of a .NET build: building, built, failed, or not built when the build stopped. */
export interface BuildProjectState {
  name: string;
  path: string;
  status: "running" | "success" | "failed" | "skipped";
}

export type BuildNode =
  | { kind: "root"; key: string; title: string; status: BuildStatus; errors: number; warnings: number }
  | { kind: "project"; key: string; project: BuildProjectState; files: BuildNode[] }
  | { kind: "file"; key: string; path: string; messages: BuildMessage[] }
  | { kind: "message"; key: string; message: BuildMessage };

export const ROOT_KEY = "build:root";

const RANK: Record<Severity, number> = { error: 0, warning: 1, info: 2, hint: 3 };

const count = (messages: readonly BuildMessage[], s: Severity) => messages.filter((m) => m.severity === s).length;

/** Root node of the tree for the current build. */
export function rootNode(title: string, status: BuildStatus, messages: readonly BuildMessage[]): BuildNode {
  return {
    kind: "root",
    key: ROOT_KEY,
    title,
    status,
    errors: count(messages, "error"),
    warnings: count(messages, "warning"),
  };
}

const dirOf = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

/**
 * Children of the root: with the projects of a .NET build, a node per project holding
 * the files under its folder; then the other files with messages (errors first) and the
 * messages about the build itself.
 */
export function rootChildren(
  messages: readonly BuildMessage[],
  projects: readonly BuildProjectState[] = [],
): BuildNode[] {
  const flat = fileNodes(messages);
  if (!projects.length) return flat;
  // The deepest project folder holding a file owns it (nested projects).
  const dirs = projects.map((p) => `${dirOf(p.path)}/`);
  const owned = new Map<number, BuildNode[]>();
  const rest: BuildNode[] = [];
  for (const n of flat) {
    let best = -1;
    if (n.kind === "file")
      dirs.forEach((d, i) => {
        if (n.path.startsWith(d) && (best < 0 || d.length > dirs[best]!.length)) best = i;
      });
    if (best < 0) rest.push(n);
    else owned.set(best, [...(owned.get(best) ?? []), n]);
  }
  const nodes: BuildNode[] = projects.map((project, i) => ({
    kind: "project",
    key: `build:project:${project.path}`,
    project,
    files: owned.get(i) ?? [],
  }));
  return [...nodes, ...rest];
}

function fileNodes(messages: readonly BuildMessage[]): BuildNode[] {
  const byFile = new Map<string, BuildMessage[]>();
  const general: BuildNode[] = [];
  messages.forEach((m, i) => {
    if (m.path) {
      let list = byFile.get(m.path);
      if (!list) byFile.set(m.path, (list = []));
      list.push(m);
    } else general.push({ kind: "message", key: `build:msg:${i}`, message: m });
  });
  const files: BuildNode[] = [...byFile].map(([path, list]) => ({
    kind: "file",
    key: `build:file:${path}`,
    path,
    messages: [...list].sort(
      (a, b) =>
        (a.range?.start.line ?? 0) - (b.range?.start.line ?? 0) ||
        (a.range?.start.col ?? 0) - (b.range?.start.col ?? 0) ||
        RANK[a.severity] - RANK[b.severity],
    ),
  }));
  const worst = (n: BuildNode) => (n.kind === "file" ? Math.min(...n.messages.map((m) => RANK[m.severity])) : 0);
  files.sort(
    (a, b) => worst(a) - worst(b) || (a.kind === "file" && b.kind === "file" ? a.path.localeCompare(b.path) : 0),
  );
  return [...files, ...general];
}

export function fileChildren(node: Extract<BuildNode, { kind: "file" }>): BuildNode[] {
  return node.messages.map((message, i) => ({ kind: "message", key: `${node.key}\u0000${i}`, message }));
}

/** `450 ms`, `3 s 120 ms`, `1 m 5 s` (IDEA's `StringUtil.formatDuration`). */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms));
  if (total < 1000) return `${total} ms`;
  const m = Math.floor(total / 60_000);
  const s = Math.floor((total % 60_000) / 1000);
  const rest = total % 1000;
  if (m > 0) return s ? `${m} m ${s} s` : `${m} m`;
  return rest ? `${s} s ${rest} ms` : `${s} s`;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Notification / root title text when a build finishes. */
export function finishSummary(result: Pick<BuildResult, "success" | "errors" | "warnings" | "durationMs">): string {
  const duration = formatDuration(result.durationMs ?? 0);
  if (result.success) {
    return result.warnings
      ? `Build completed successfully with ${plural(result.warnings, "warning")} in ${duration}`
      : `Build completed successfully in ${duration}`;
  }
  if (!result.errors) return "Build failed";
  return result.warnings
    ? `Build failed: ${plural(result.errors, "error")}, ${plural(result.warnings, "warning")}`
    : `Build failed: ${plural(result.errors, "error")}`;
}

/** Files that received build diagnostics (to clear before the next build). */
export function messageFiles(messages: readonly BuildMessage[]): string[] {
  return [...new Set(messages.filter((m) => m.path && m.range).map((m) => m.path!))];
}

/** Children of a node below the root. */
export function nodeChildren(n: BuildNode): BuildNode[] | undefined {
  return n.kind === "project" ? n.files : n.kind === "file" ? fileChildren(n) : undefined;
}

function* walk(nodes: readonly BuildNode[]): Generator<BuildNode> {
  for (const n of nodes) {
    yield n;
    const children = nodeChildren(n);
    if (children) yield* walk(children);
  }
}

/** The node with `key` in the tree of `messages` (root excluded). */
export function findNode(
  messages: readonly BuildMessage[],
  key: string,
  projects: readonly BuildProjectState[] = [],
): BuildNode | undefined {
  for (const n of walk(rootChildren(messages, projects))) if (n.key === key) return n;
  return undefined;
}

/** The first error in tree order, selected (and navigated to) when a build fails. */
export function firstError(
  messages: readonly BuildMessage[],
  projects: readonly BuildProjectState[] = [],
): Extract<BuildNode, { kind: "message" }> | undefined {
  for (const n of walk(rootChildren(messages, projects)))
    if (n.kind === "message" && n.message.severity === "error") return n;
  return undefined;
}

/** The final state of the projects when a build ends: unfinished ones were built (success) or not. */
export function finishProjects(projects: readonly BuildProjectState[], success: boolean): BuildProjectState[] {
  return projects.map((p) => (p.status === "running" ? { ...p, status: success ? "success" : "skipped" } : p));
}
