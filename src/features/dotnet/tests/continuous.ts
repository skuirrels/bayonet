/**
 * Continuous testing (TST-3): after a save (or a successful build), rerun the
 * tests affected by what changed. A .NET source file's members are compared
 * with their text when it was opened or last saved; for each changed member,
 * the tests whose method contains it, or that reach it through Roslyn's find
 * references (a few calls deep), are run. Tests not discovered yet mean all
 * tests run, once.
 */
import { create } from "zustand";
import { commands, type OutlineItem, type Pos, type Range, type TestSelection } from "@/bindings";
import { onEditorEvent, openEditorPaths } from "@/editor/api";
import { withDocument } from "@/features/code-insight/ipc";
import { useBuildRun } from "@/features/build/store";
import { tx } from "@/platform/i18n";
import { call, isTauri, nextRequestId } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { runTests, testsIn, useUnitTests } from "./store";

export type ContinuousMode = "off" | "save" | "build";

const DOTNET_SOURCE = /\.(cs|vb|fs)$/i;
/** Saves within this long (Save All, format on save) are one change. */
const SAVE_DELAY = 800;
/** Calls followed from a changed member towards the tests that reach it. */
const MAX_DEPTH = 3;
/** Reference searches per rerun. */
const MAX_SEARCHES = 40;

/** A member of a source file: a method, property, constructor, field, … */
export interface Member {
  /** Container names, name and signature: stable across edits elsewhere in the file. */
  key: string;
  name: string;
  range: Range;
  /** Where the name is: what find references starts from. */
  at: Pos;
  text: string;
}

const CONTAINERS = new Set<OutlineItem["kind"]>([
  "file",
  "module",
  "namespace",
  "package",
  "class",
  "interface",
  "struct",
  "enum",
  "trait",
]);

/** `range` of `text` (0-based lines, UTF-16 columns). */
function slice(lines: readonly string[], range: Range): string {
  const { start, end } = range;
  if (start.line === end.line) return (lines[start.line] ?? "").slice(start.col, end.col);
  const out = [(lines[start.line] ?? "").slice(start.col)];
  for (let l = start.line + 1; l < end.line; l++) out.push(lines[l] ?? "");
  out.push((lines[end.line] ?? "").slice(0, end.col));
  return out.join("\n");
}

/** The members of a file, by key; overloads with the same signature get an index. */
export function membersOf(text: string, outline: readonly OutlineItem[]): Map<string, Member> {
  const lines = text.split(/\r?\n/);
  const out = new Map<string, Member>();
  const walk = (items: readonly OutlineItem[], path: string) => {
    for (const item of items) {
      const here = `${path}${item.name}`;
      if (CONTAINERS.has(item.kind) && item.children.length) {
        walk(item.children, `${here}.`);
        continue;
      }
      if (CONTAINERS.has(item.kind)) continue;
      let key = `${here}(${item.detail ?? ""})`;
      for (let i = 2; out.has(key); i++) key = `${here}(${item.detail ?? ""})#${i}`;
      out.set(key, {
        key,
        name: item.name,
        range: item.range,
        at: item.selection_range.start,
        text: slice(lines, item.range).replace(/[ \t]+$/gm, ""),
      });
    }
  };
  walk(outline, "");
  return out;
}

/** Members added or edited between two versions of a file (removed ones have no tests left to run). */
export function changedMembers(before: ReadonlyMap<string, Member>, after: ReadonlyMap<string, Member>): Member[] {
  return [...after.values()].filter((m) => before.get(m.key)?.text !== m.text);
}

/** The innermost member containing 0-based `line`. */
export function memberAt(members: Iterable<Member>, line: number): Member | undefined {
  let best: Member | undefined;
  for (const m of members)
    if (m.range.start.line <= line && line <= m.range.end.line)
      if (!best || m.range.start.line >= best.range.start.line) best = m;
  return best;
}

/** Tests (by project) whose method is `member` of `file`. */
export function testsInMember(file: string, member: Member): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  for (const { project, test } of testsIn(file)) {
    // Test lines are 1-based.
    const line = (test.line ?? 0) - 1;
    if (line < member.range.start.line || line > member.range.end.line) continue;
    let uids = out.get(project);
    if (!uids) out.set(project, (uids = new Set()));
    uids.add(test.uid);
  }
  return out;
}

const merge = (into: Map<string, Set<string>>, from: Map<string, Set<string>>) => {
  for (const [project, uids] of from) {
    const set = into.get(project);
    if (set) for (const u of uids) set.add(u);
    else into.set(project, new Set(uids));
  }
};

export interface Affected {
  selection: TestSelection[];
  count: number;
  /** A search missed usages (indexing, or the search budget ran out). */
  incomplete: boolean;
}

/** The tests that run `changes` (members of their files), directly or through calls. */
export async function affectedTests(changes: ReadonlyArray<{ path: string; member: Member }>): Promise<Affected> {
  const found = new Map<string, Set<string>>();
  const outlines = new Map<string, Promise<Map<string, Member>>>();
  const seen = new Set<string>();
  let searches = 0;
  let incomplete = false;
  const membersIn = (path: string) => {
    let p = outlines.get(path);
    if (!p) {
      p = withDocument(path, async (doc) => {
        const [info, outline] = await Promise.all([call(commands.docSnapshot(doc)), call(commands.docOutline(doc))]);
        return membersOf(info.text, outline);
      }).catch(() => new Map<string, Member>());
      outlines.set(path, p);
    }
    return p;
  };
  const visit = async (path: string, member: Member, depth: number): Promise<void> => {
    const id = `${path}#${member.key}`;
    if (seen.has(id)) return;
    seen.add(id);
    const direct = testsInMember(path, member);
    if (direct.size) {
      merge(found, direct);
      return;
    }
    if (depth >= MAX_DEPTH) return;
    if (searches >= MAX_SEARCHES) {
      incomplete = true;
      return;
    }
    searches++;
    const refs = await withDocument(path, (doc) =>
      call(commands.lspReferences(nextRequestId(), doc, member.at, false, true)),
    ).catch(() => null);
    if (!refs) {
      incomplete = true;
      return;
    }
    if (refs.incomplete) incomplete = true;
    const callers: Array<Promise<void>> = [];
    for (const loc of refs.locations) {
      if (!DOTNET_SOURCE.test(loc.path)) continue;
      callers.push(
        membersIn(loc.path).then((members) => {
          const caller = memberAt(members.values(), loc.range.start.line);
          return caller ? visit(loc.path, caller, depth + 1) : undefined;
        }),
      );
    }
    await Promise.all(callers);
  };
  for (const { path, member } of changes) await visit(path, member, 0);
  const selection = [...found].map(([project, uids]) => ({ project, uids: [...uids] }));
  return { selection, count: selection.reduce((n, s) => n + s.uids.length, 0), incomplete };
}

interface ContinuousState {
  mode: ContinuousMode;
  /** Changed members waiting for the next rerun. */
  pending: number;
  analysing: boolean;
  /** What the last rerun did. */
  last: string | null;
}

export const useContinuous = create<ContinuousState>(() => ({
  mode: "off",
  pending: 0,
  analysing: false,
  last: null,
}));

const storageKey = () => `openidea.dotnet.continuousTesting:${useProject.getState().project?.root ?? ""}`;

function storedMode(): ContinuousMode {
  try {
    const v = localStorage.getItem(storageKey());
    return v === "save" || v === "build" ? v : "off";
  } catch {
    return "off";
  }
}

export function setContinuousMode(mode: ContinuousMode) {
  useContinuous.setState({ mode, last: null });
  try {
    localStorage.setItem(storageKey(), mode);
  } catch {
    // private window: the choice lasts for the session
  }
  if (mode === "off") pending.clear();
  useContinuous.setState({ pending: pending.size });
}

/** Members of each open source file as last run (opened or saved). */
const baselines = new Map<string, Map<string, Member>>();
/** Changed and not rerun yet, by `<path>#<key>`. */
const pending = new Map<string, { path: string; member: Member }>();

async function snapshot(path: string): Promise<Map<string, Member> | null> {
  try {
    return await withDocument(path, async (doc) => {
      const [info, outline] = await Promise.all([call(commands.docSnapshot(doc)), call(commands.docOutline(doc))]);
      return membersOf(info.text, outline);
    });
  } catch {
    return null;
  }
}

async function remember(path: string) {
  if (baselines.has(path)) return;
  const members = await snapshot(path);
  if (members && !baselines.has(path)) baselines.set(path, members);
}

async function saved(path: string) {
  const before = baselines.get(path);
  const after = await snapshot(path);
  if (!after) return;
  baselines.set(path, after);
  if (!before) return;
  for (const member of changedMembers(before, after)) pending.set(`${path}#${member.key}`, { path, member });
  useContinuous.setState({ pending: pending.size });
}

let running = false;
let again = false;

/** Reruns the tests of the pending changes (after the current run, if one is busy). */
export async function rerunAffected(): Promise<void> {
  if (!pending.size) return;
  if (running || useUnitTests.getState().busy) {
    again = true;
    return;
  }
  running = true;
  const changes = [...pending.values()];
  pending.clear();
  useContinuous.setState({ pending: 0, analysing: true });
  try {
    const known = useUnitTests.getState().projects.some((p) => p.tests.size > 0);
    if (!known) {
      useContinuous.setState({ last: tx`Tests are not discovered yet: running all of them` });
      await runTests();
      return;
    }
    const affected = await affectedTests(changes);
    const names = [...new Set(changes.map((c) => c.member.name))].join(", ");
    if (!affected.count) {
      useContinuous.setState({ last: tx`No tests reach ${names}` });
      return;
    }
    const count = affected.count;
    useContinuous.setState({ last: tx`Rerunning ${count} tests affected by ${names}`, analysing: false });
    await runTests(affected.selection);
  } finally {
    running = false;
    useContinuous.setState({ analysing: false });
    if (again) {
      again = false;
      void rerunAffected();
    }
  }
}

/** Watches saves and builds; returns the disposer. */
export function watchContinuous(): () => void {
  if (!isTauri()) return () => {};
  useContinuous.setState({ mode: storedMode(), pending: 0, last: null });
  for (const path of openEditorPaths()) if (DOTNET_SOURCE.test(path)) void remember(path);
  let timer: ReturnType<typeof setTimeout> | null = null;
  const unsubEditor = onEditorEvent((e) => {
    if (e.type === "docOpened" && DOTNET_SOURCE.test(e.path)) void remember(e.path);
    else if (e.type === "docClosed") baselines.delete(e.path);
    else if (e.type === "docSaved" && DOTNET_SOURCE.test(e.path)) {
      if (useContinuous.getState().mode === "off") {
        baselines.delete(e.path);
        void remember(e.path);
        return;
      }
      void saved(e.path).then(() => {
        if (useContinuous.getState().mode !== "save") return;
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => void rerunAffected(), SAVE_DELAY);
      });
    }
  });
  const unsubBuild = useBuildRun.subscribe((s, prev) => {
    if (useContinuous.getState().mode === "build" && prev.status === "running" && s.status === "success")
      void rerunAffected();
  });
  // A rerun asked for while tests ran.
  const unsubTests = useUnitTests.subscribe((s, prev) => {
    if (prev.busy && !s.busy && again && !running) {
      again = false;
      void rerunAffected();
    }
  });
  return () => {
    if (timer) clearTimeout(timer);
    unsubEditor();
    unsubBuild();
    unsubTests();
    baselines.clear();
    pending.clear();
  };
}
