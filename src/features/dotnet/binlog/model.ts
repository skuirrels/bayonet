/**
 * The binary log viewer's tree (BLD-3): errors and warnings first, then the
 * projects the build started at the top (a solution, or the project built),
 * each with its targets, the tasks they ran and the projects it built in turn.
 * Pure, so the viewer and the tests share it.
 */
import type { Binlog, BinlogMessage, BinlogProject, BinlogTarget, BinlogTask } from "@/bindings";

export type BinlogNode =
  | { kind: "diagnostics"; key: string; items: BinlogMessage[] }
  | { kind: "messages"; key: string; items: BinlogMessage[] }
  | { kind: "project"; key: string; project: BinlogProject }
  | { kind: "target"; key: string; target: BinlogTarget; project: BinlogProject }
  | { kind: "task"; key: string; task: BinlogTask; target: BinlogTarget }
  | { kind: "message"; key: string; message: BinlogMessage };

export const DIAGNOSTICS_KEY = "diagnostics";
export const MESSAGES_KEY = "messages";

export interface BinlogIndex {
  log: Binlog;
  /** Child project instances by parent project id. */
  children: Map<number, BinlogProject[]>;
  byId: Map<number, BinlogProject>;
  /** Show skipped targets (condition false, up to date). */
  showSkipped: boolean;
}

export function indexLog(log: Binlog, showSkipped = false): BinlogIndex {
  const byId = new Map(log.projects.map((p) => [p.id, p]));
  const children = new Map<number, BinlogProject[]>();
  for (const p of log.projects) {
    if (p.parent == null || !byId.has(p.parent)) continue;
    let list = children.get(p.parent);
    if (!list) children.set(p.parent, (list = []));
    list.push(p);
  }
  return { log, children, byId, showSkipped };
}

const projectKey = (p: BinlogProject) => `p${p.id}`;

const messageNodes = (owner: string, items: readonly BinlogMessage[]): BinlogNode[] =>
  items.map((message, i) => ({ kind: "message", key: `${owner}/m${i}`, message }));

export function rootNodes(ix: BinlogIndex): BinlogNode[] {
  const { log } = ix;
  const out: BinlogNode[] = [];
  if (log.diagnostics.length) out.push({ kind: "diagnostics", key: DIAGNOSTICS_KEY, items: log.diagnostics });
  for (const p of log.projects)
    if (p.parent == null || !ix.byId.has(p.parent)) out.push({ kind: "project", key: projectKey(p), project: p });
  if (log.messages.length) out.push({ kind: "messages", key: MESSAGES_KEY, items: log.messages });
  return out;
}

export function childNodes(ix: BinlogIndex, n: BinlogNode): BinlogNode[] | undefined {
  switch (n.kind) {
    case "diagnostics":
    case "messages":
      return messageNodes(n.key, n.items);
    case "project": {
      const p = n.project;
      const targets: BinlogNode[] = [];
      p.targets.forEach((target, i) => {
        if (target.skipped && !ix.showSkipped) return;
        targets.push({ kind: "target", key: `${n.key}/t${i}`, target, project: p });
      });
      const projects: BinlogNode[] = (ix.children.get(p.id) ?? []).map((c) => ({
        kind: "project",
        key: projectKey(c),
        project: c,
      }));
      return [...targets, ...projects, ...messageNodes(n.key, p.messages)];
    }
    case "target":
      return [
        ...n.target.tasks.map((task, i): BinlogNode => ({
          kind: "task",
          key: `${n.key}/k${i}`,
          task,
          target: n.target,
        })),
        ...messageNodes(n.key, n.target.messages),
      ];
    case "task":
      return messageNodes(n.key, n.task.messages);
    case "message":
      return undefined;
  }
}

export function hasChildren(ix: BinlogIndex, n: BinlogNode): boolean {
  switch (n.kind) {
    case "message":
      return false;
    case "task":
      return n.task.messages.length > 0;
    case "target":
      return n.target.tasks.length > 0 || n.target.messages.length > 0;
    case "project":
      return (
        n.project.messages.length > 0 ||
        (ix.children.get(n.project.id)?.length ?? 0) > 0 ||
        n.project.targets.some((t) => ix.showSkipped || !t.skipped)
      );
    default:
      return n.items.length > 0;
  }
}

/** Keys of the nodes above `key`, outermost first (to expand before revealing it). */
export function ancestorKeys(ix: BinlogIndex, key: string): string[] {
  const parts = key.split("/");
  const head = parts[0]!;
  const out: string[] = [];
  if (head.startsWith("p")) {
    // The chain of parent projects.
    const chain: string[] = [];
    let p = ix.byId.get(Number(head.slice(1)));
    while (p && p.parent != null && ix.byId.has(p.parent)) {
      p = ix.byId.get(p.parent);
      if (p) chain.unshift(projectKey(p));
    }
    out.push(...chain);
  }
  for (let i = 1; i < parts.length; i++) out.push(parts.slice(0, i).join("/"));
  return out;
}

/** The node with `key`. */
export function findBinlogNode(ix: BinlogIndex, key: string): BinlogNode | undefined {
  const parts = key.split("/");
  let nodes = rootNodes(ix);
  let node: BinlogNode | undefined;
  const head = parts[0]!;
  if (head.startsWith("p")) {
    const p = ix.byId.get(Number(head.slice(1)));
    if (!p) return undefined;
    node = { kind: "project", key: head, project: p };
  } else node = nodes.find((n) => n.key === head);
  for (let i = 1; node && i < parts.length; i++) {
    const want = parts.slice(0, i + 1).join("/");
    // Skipped targets keep their index, so look them up even when hidden.
    nodes = childNodes({ ...ix, showSkipped: true }, node) ?? [];
    node = nodes.find((n) => n.key === want);
  }
  return node;
}

export interface SearchHit {
  key: string;
  /** `Fixture.Api.csproj › CoreCompile › Csc`. */
  location: string;
  text: string;
  kind: BinlogNode["kind"];
}

/** Projects, targets, tasks and messages whose name or text contains `query` (case-insensitive). */
export function searchLog(ix: BinlogIndex, query: string, limit = 500): SearchHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const hits: SearchHit[] = [];
  const has = (s: string | null | undefined) => !!s && s.toLowerCase().includes(q);
  const messages = (owner: string, location: string, items: readonly BinlogMessage[]) => {
    items.forEach((m, i) => {
      if (hits.length < limit && (has(m.text) || has(m.code) || has(m.file)))
        hits.push({ key: `${owner}/m${i}`, location, text: m.text, kind: "message" });
    });
  };
  for (const p of ix.log.projects) {
    if (hits.length >= limit) break;
    const pk = projectKey(p);
    if (has(p.name) || has(p.file)) hits.push({ key: pk, location: p.name, text: p.file, kind: "project" });
    messages(pk, p.name, p.messages);
    p.targets.forEach((t, i) => {
      const tk = `${pk}/t${i}`;
      const tl = `${p.name} › ${t.name}`;
      if (hits.length < limit && has(t.name)) hits.push({ key: tk, location: p.name, text: t.name, kind: "target" });
      messages(tk, tl, t.messages);
      t.tasks.forEach((k, j) => {
        const kk = `${tk}/k${j}`;
        if (hits.length < limit && (has(k.name) || has(k.commandLine)))
          hits.push({ key: kk, location: tl, text: has(k.name) ? k.name : (k.commandLine ?? k.name), kind: "task" });
        messages(kk, `${tl} › ${k.name}`, k.messages);
      });
    });
  }
  messages(MESSAGES_KEY, "Build", ix.log.messages);
  return hits.slice(0, limit);
}

/** The slowest targets of the build, for the summary (skipped ones excluded). */
export function slowestTargets(log: Binlog, count = 10): { project: string; target: BinlogTarget }[] {
  const all: { project: string; target: BinlogTarget }[] = [];
  for (const p of log.projects) for (const t of p.targets) if (!t.skipped) all.push({ project: p.name, target: t });
  return all.sort((a, b) => b.target.durationMs - a.target.durationMs).slice(0, count);
}

/** `Fixture.Api.csproj (net10.0, Debug)`'s parenthesis, or `""`. */
export function projectContext(p: BinlogProject): string {
  const parts = [p.targetFramework, p.configuration].filter((s): s is string => !!s);
  return parts.length ? parts.join(", ") : "";
}

/** Start and length of a node's bar on the build's timeline, as fractions 0..1. */
export function timelineBar(startMs: number, durationMs: number, totalMs: number): { left: number; width: number } {
  if (totalMs <= 0) return { left: 0, width: 0 };
  const left = Math.min(1, Math.max(0, startMs / totalMs));
  return { left, width: Math.min(1 - left, Math.max(0, durationMs / totalMs)) };
}
