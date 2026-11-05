/** Unit Tests window state: test projects, their tests and the running discovery or run. */
import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import { commands } from "@/bindings";
import type { RequestId, TestEvent, TestNode, TestSelection } from "@/bindings";
import { call, isCancelled, isTauri, nextRequestId } from "@/platform/ipc";
import { mergeTest, type ProjectTests } from "./model";
import { collectTestCoverage } from "./coverage";
import { tx } from "@/platform/i18n";
import { attachDotnetDebugger } from "../attach";

const MAX_OUTPUT = 2000;

export interface UnitTestsState {
  /** By project path, in name order. */
  projects: ProjectTests[];
  busy: "discover" | "run" | null;
  request: RequestId | null;
  /** Build and test-process output of the last discovery or run. */
  output: string[];
  error: string | null;
  /** Selected tree keys and the lead. */
  selected: ReadonlySet<string>;
  lead: string | null;
  expanded: ReadonlySet<string>;
  query: string;
  failedOnly: boolean;
  /** Bumped when any test changes (gutter markers). */
  version: number;
}

const initial = (): UnitTestsState => ({
  projects: [],
  busy: null,
  request: null,
  output: [],
  error: null,
  selected: new Set(),
  lead: null,
  expanded: new Set(),
  query: "",
  failedOnly: false,
  version: 0,
});

export const useUnitTests = create<UnitTestsState>(initial);

export function resetUnitTests() {
  const { request } = useUnitTests.getState();
  if (request !== null) void commands.cancelRequest(request);
  useUnitTests.setState(initial());
}

const projectName = (path: string) => (path.split(/[\\/]/).pop() ?? path).replace(/\.[^.]+proj$/i, "");

function apply(e: TestEvent) {
  useUnitTests.setState((s) => {
    switch (e.kind) {
      case "output": {
        const output = s.output.length >= MAX_OUTPUT ? s.output.slice(-MAX_OUTPUT / 2) : s.output.slice();
        output.push(e.line);
        return { output };
      }
      case "phase": {
        if (!e.project) return e.phase === "failed" ? { error: e.message } : {};
        const done = e.phase === "finished" || e.phase === "failed";
        return {
          projects: s.projects.map((p) =>
            p.path === e.project
              ? {
                  ...p,
                  busy: !done,
                  message: e.phase === "failed" ? e.message : null,
                }
              : p,
          ),
        };
      }
      case "tests": {
        let known = s.projects.find((p) => p.path === e.project);
        const projects = known
          ? s.projects
          : [
              ...s.projects,
              (known = {
                path: e.project,
                name: projectName(e.project),
                tests: new Map(),
                message: null,
                busy: true,
              }),
            ];
        const tests = new Map(known.tests);
        for (const t of e.tests) tests.set(t.uid, mergeTest(tests.get(t.uid), t));
        return {
          projects: projects
            .map((p) => (p.path === e.project ? { ...p, tests } : p))
            .sort((a, b) => a.name.localeCompare(b.name)),
          version: s.version + 1,
        };
      }
      case "coverage":
      case "debug":
        return {};
    }
  });
}

/** Lists the test projects (without their tests) so the window is not empty before discovery. */
export async function loadProjects(): Promise<void> {
  if (!isTauri()) return;
  try {
    const list = await call(commands.dotnetTestProjects());
    useUnitTests.setState((s) => ({
      projects: list.map(
        (p) =>
          s.projects.find((o) => o.path === p.path) ?? {
            path: p.path,
            name: p.name,
            tests: new Map(),
            message: null,
            busy: false,
          },
      ),
    }));
  } catch (e) {
    useUnitTests.setState({
      error: e instanceof Error ? e.message : String(e),
    });
  }
}

/** Attaches netcoredbg to a debug run's test app, which waits for it before running. */
async function attachDebugger(project: string, pid: number) {
  const { useProject } = await import("@/platform/project");
  const cwd = useProject.getState().project?.root ?? project.replace(/[\\/][^\\/]*$/, "");
  await attachDotnetDebugger(pid, tx`Debug tests: ${projectName(project)}`, cwd);
}

async function start(run: boolean, selection: TestSelection[], coverage = false, debug = false): Promise<void> {
  if (!isTauri() || useUnitTests.getState().busy) return;
  const request = nextRequestId();
  const channel = new Channel<TestEvent>();
  // Cobertura reports by test project, from a coverage run.
  const reports = new Map<string, string>();
  channel.onmessage = (e) => {
    if (e.kind === "coverage") reports.set(e.project, e.report);
    else if (e.kind === "debug")
      void attachDebugger(e.project, e.pid).catch((err: unknown) =>
        useUnitTests.setState({ error: err instanceof Error ? err.message : String(err) }),
      );
    else apply(e);
  };
  const wanted = new Map(selection.map((t) => [t.project, new Set(t.uids)]));
  useUnitTests.setState((s) => ({
    busy: run ? "run" : "discover",
    request,
    output: [],
    error: null,
    // A run starts its tests afresh; an empty uid set means the whole project.
    projects: s.projects.map((p) => {
      const uids = selection.length === 0 ? new Set<string>() : wanted.get(p.path);
      if (!uids) return p;
      if (!run) return { ...p, busy: true, message: null };
      const tests = new Map<string, TestNode>();
      for (const [uid, t] of p.tests)
        tests.set(
          uid,
          uids.size === 0 || uids.has(uid)
            ? {
                ...t,
                state: "discovered",
                message: null,
                stackTrace: null,
                durationMs: null,
                output: null,
                expected: null,
                actual: null,
              }
            : t,
        );
      return { ...p, tests, busy: true, message: null };
    }),
    version: s.version + 1,
  }));
  let finished = false;
  try {
    await call(commands.dotnetTests(request, run, coverage, debug, selection, channel));
    finished = true;
  } catch (e) {
    if (!isCancelled(e))
      useUnitTests.setState({
        error: e instanceof Error ? e.message : String(e),
      });
  } finally {
    useUnitTests.setState((s) => ({
      busy: null,
      request: null,
      projects: s.projects.map((p) => (p.busy ? { ...p, busy: false } : p)),
      version: s.version + 1,
    }));
  }
  if (coverage && finished) {
    const ran = useUnitTests
      .getState()
      .projects.filter((p) => (selection.length === 0 || wanted.has(p.path)) && !p.message)
      .map((p) => ({ path: p.path, name: p.name }));
    await collectTestCoverage(reports, ran);
  }
}

/** Builds and discovers the tests of every test project. */
export const discoverTests = () => start(false, []);

/** Builds and runs `selection` (every test when empty). */
export const runTests = (selection: TestSelection[] = []) => start(true, selection);

/** Builds and runs `selection` (every test when empty) under the debugger (DBG-5). */
export const debugTests = (selection: TestSelection[] = []) => start(true, selection, false, true);

/** Builds and runs `selection` (every test when empty) with Microsoft Code Coverage. */
export const runTestsWithCoverage = (selection: TestSelection[] = []) => start(true, selection, true);

export function cancelTests() {
  const { request } = useUnitTests.getState();
  if (request !== null) void commands.cancelRequest(request);
}

/** Tests declared in `file`, for the gutter. */
export function testsIn(file: string): Array<{ project: string; test: TestNode }> {
  const out: Array<{ project: string; test: TestNode }> = [];
  for (const p of useUnitTests.getState().projects)
    for (const t of p.tests.values()) if (t.file === file) out.push({ project: p.path, test: t });
  return out;
}
