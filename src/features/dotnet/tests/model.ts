/**
 * Unit Tests window model: the tests of each project as a tree of namespaces,
 * classes, methods (data-driven rows grouped under their method) and tests.
 */
import type { TestNode, TestSelection, TestState } from "@/bindings";
import type { TestStatus } from "@/features/run/testing";

export interface ProjectTests {
  path: string;
  name: string;
  tests: ReadonlyMap<string, TestNode>;
  /** Why discovery or the run failed for this project. */
  message: string | null;
  busy: boolean;
}

export type UtNode =
  | {
      kind: "project";
      key: string;
      name: string;
      project: string;
      children: UtNode[];
    }
  | {
      kind: "namespace";
      key: string;
      name: string;
      project: string;
      children: UtNode[];
    }
  | {
      kind: "class";
      key: string;
      name: string;
      project: string;
      typeName: string;
      children: UtNode[];
    }
  | {
      kind: "method";
      key: string;
      name: string;
      project: string;
      typeName: string;
      children: UtNode[];
    }
  | {
      kind: "test";
      key: string;
      name: string;
      project: string;
      test: TestNode;
      children: UtNode[];
    };

/** The tree's status of a test state; `null` before it ran. */
export function statusOf(state: TestState): TestStatus | null {
  switch (state) {
    case "passed":
      return "passed";
    case "failed":
    case "error":
    case "timedOut":
      return "failed";
    case "skipped":
    case "cancelled":
      return "skipped";
    case "inProgress":
      return "running";
    default:
      return null;
  }
}

/** Aggregated status: failed wins, then running, then passed; skipped only when all were. */
export function nodeStatus(node: UtNode): TestStatus | null {
  if (node.kind === "test") return statusOf(node.test.state);
  let any = false;
  let running = false;
  let passed = false;
  for (const c of node.children) {
    const s = nodeStatus(c);
    if (s === "failed") return "failed";
    if (s === "running") running = true;
    else if (s === "passed") passed = true;
    if (s) any = true;
  }
  if (running) return "running";
  if (!any) return null;
  return passed ? "passed" : "skipped";
}

export function leaves(node: UtNode): TestNode[] {
  return node.kind === "test" ? [node.test] : node.children.flatMap(leaves);
}

const methodBase = (t: TestNode) => (t.method ?? t.displayName).split("(")[0]!;

/** What a test row shows: its arguments under a method node, else the method name. */
function leafName(t: TestNode, grouped: boolean): string {
  let name = t.displayName;
  if (t.typeName && name.startsWith(`${t.typeName}.`)) name = name.slice(t.typeName.length + 1);
  if (!grouped) return name.includes("(") ? name : methodBase(t) || name;
  const open = name.indexOf("(");
  return open >= 0 ? name.slice(open) : name;
}

const byName = <T extends { name: string }>(a: T, b: T) => a.name.localeCompare(b.name);

/** `query` keeps tests whose name, method or type contains it; `failedOnly` keeps failed ones. */
export function buildTree(projects: readonly ProjectTests[], query = "", failedOnly = false): UtNode[] {
  const q = query.trim().toLowerCase();
  const keep = (t: TestNode) =>
    (!failedOnly || statusOf(t.state) === "failed") &&
    (!q || [t.displayName, t.method ?? "", t.typeName ?? ""].some((s) => s.toLowerCase().includes(q)));
  const out: UtNode[] = [];
  for (const p of projects) {
    const tests = [...p.tests.values()].filter(keep);
    if ((q || failedOnly) && tests.length === 0) continue;
    const namespaces = new Map<string, Map<string, TestNode[]>>();
    for (const t of tests) {
      const type = t.typeName ?? "";
      const dot = type.lastIndexOf(".");
      const ns = dot < 0 ? "" : type.slice(0, dot);
      const classes = namespaces.get(ns) ?? new Map<string, TestNode[]>();
      namespaces.set(ns, classes);
      classes.set(type, [...(classes.get(type) ?? []), t]);
    }
    const projectKey = `p:${p.path}`;
    const nsNodes: UtNode[] = [...namespaces].map(([ns, classes]) => {
      const nsKey = `${projectKey}\u0000n:${ns}`;
      const classNodes: UtNode[] = [...classes].map(([type, members]) => {
        const classKey = `${nsKey}\u0000c:${type}`;
        const methods = new Map<string, TestNode[]>();
        for (const t of members) methods.set(methodBase(t), [...(methods.get(methodBase(t)) ?? []), t]);
        const children: UtNode[] = [...methods].map(([m, rows]): UtNode => {
          if (rows.length === 1)
            return {
              kind: "test",
              key: `t:${p.path}\u0000${rows[0]!.uid}`,
              name: leafName(rows[0]!, false),
              project: p.path,
              test: rows[0]!,
              children: [],
            };
          const methodKey = `${classKey}\u0000m:${m}`;
          return {
            kind: "method",
            key: methodKey,
            name: m,
            project: p.path,
            typeName: type,
            children: rows
              .map((t): UtNode => ({
                kind: "test",
                key: `t:${p.path}\u0000${t.uid}`,
                name: leafName(t, true),
                project: p.path,
                test: t,
                children: [],
              }))
              .sort(byName),
          };
        });
        const simple = type.slice(type.lastIndexOf(".") + 1) || "(no class)";
        return {
          kind: "class",
          key: classKey,
          name: simple,
          project: p.path,
          typeName: type,
          children: children.sort(byName),
        };
      });
      return {
        kind: "namespace",
        key: nsKey,
        name: ns || "(global)",
        project: p.path,
        children: classNodes.sort(byName),
      };
    });
    out.push({
      kind: "project",
      key: projectKey,
      name: p.name,
      project: p.path,
      children: nsNodes.sort(byName),
    });
  }
  return out;
}

/** What running `nodes` selects: whole projects, else the tests under each node, per project. */
export function selectionOf(nodes: readonly UtNode[]): TestSelection[] {
  const whole = new Set(nodes.filter((n) => n.kind === "project").map((n) => n.project));
  const uids = new Map<string, Set<string>>();
  for (const n of nodes) {
    if (whole.has(n.project)) continue;
    const set = uids.get(n.project) ?? new Set<string>();
    for (const t of leaves(n)) set.add(t.uid);
    uids.set(n.project, set);
  }
  return [
    ...[...whole].map((project) => ({ project, uids: [] })),
    ...[...uids].filter(([, s]) => s.size).map(([project, s]) => ({ project, uids: [...s] })),
  ];
}

/** Merges a test update into what is known: a run's nodes may omit the location discovery found. */
export function mergeTest(old: TestNode | undefined, next: TestNode): TestNode {
  if (!old) return next;
  return {
    ...next,
    typeName: next.typeName ?? old.typeName,
    method: next.method ?? old.method,
    file: next.file ?? old.file,
    line: next.line ?? old.line,
  };
}

export interface TestCounts {
  total: number;
  passed: number;
  failed: number;
  skipped: number;
}

export function countTests(projects: readonly ProjectTests[]): TestCounts {
  const c: TestCounts = { total: 0, passed: 0, failed: 0, skipped: 0 };
  for (const p of projects)
    for (const t of p.tests.values()) {
      c.total++;
      const s = statusOf(t.state);
      if (s === "passed") c.passed++;
      else if (s === "failed") c.failed++;
      else if (s === "skipped") c.skipped++;
    }
  return c;
}

/** `in /s/T.cs:line 12` (and `at … in …:line N`) locations in a .NET stack trace. */
export function stackLocations(trace: string): Array<{ text: string; path: string | null; line: number | null }> {
  return trace.split(/\r?\n/).map((text) => {
    const m = / in (.+):line (\d+)\s*$/.exec(text);
    return { text, path: m ? m[1]! : null, line: m ? Number(m[2]) : null };
  });
}
