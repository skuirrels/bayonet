/**
 * Source generators in the Solution Explorer (CI-17): a C# or VB project's
 * Dependencies › Analyzers lists its analyser assemblies and, below each, the
 * files its generators write (`<assembly>/<generator type>/<hint name>`).
 *
 * Roslyn keeps generated sources in memory, so the backend compiles the
 * project once more into the IDE's cache to get them. That runs when the node
 * is first expanded, and again when the project's sources change while it is
 * expanded (debounced). Go to Declaration into generated code opens Roslyn's
 * own copy (`roslyn-source-generated://`) instead.
 */
import { create } from "zustand";
import { commands, type GeneratedSources } from "@/bindings";
import { call } from "@/platform/ipc";
import { isSameOrInside, parentOf } from "@/features/project-tree/paths";
import { pathOfKey } from "@/features/project-tree/model";
import { reloadLoadedDirsUnder, useProjectTree } from "@/features/project-tree/store";

/** Project files whose analysers and generators can be listed. */
export const hasGenerators = (project: string) => /\.(csproj|vbproj)$/i.test(project);

interface GeneratedState {
  /** Cache folder per project file. */
  dirs: Readonly<Record<string, string>>;
  /** Last result per project file. */
  results: Readonly<Record<string, GeneratedSources>>;
  running: ReadonlySet<string>;
}

export const useGenerated = create<GeneratedState>(() => ({ dirs: {}, results: {}, running: new Set() }));

const get = () => useGenerated.getState();

/** Projects whose sources changed since their last side compile. */
const stale = new Set<string>();
let queue: Promise<void> = Promise.resolve();
let epoch = 0;

/** Looks up (and creates) the cache folders of `projects`; returns them by project. */
export async function loadGeneratedDirs(projects: readonly string[]): Promise<Record<string, string>> {
  const wanted = projects.filter(hasGenerators);
  const missing = wanted.filter((p) => !get().dirs[p]);
  if (missing.length) {
    const dirs = await call(commands.dotnetGeneratedDirs(missing));
    const next = { ...get().dirs };
    missing.forEach((p, i) => {
      if (dirs[i]) next[p] = dirs[i]!;
    });
    useGenerated.setState({ dirs: next });
  }
  return get().dirs;
}

/** Compiles `project` into the cache (one at a time) and refreshes its listing. */
export function generate(project: string): Promise<void> {
  if (get().running.has(project)) return queue;
  const at = epoch;
  stale.delete(project);
  useGenerated.setState((s) => ({ running: new Set(s.running).add(project) }));
  queue = queue.then(async () => {
    try {
      const result = await call(commands.dotnetGeneratedSources(project));
      if (at !== epoch) return;
      useGenerated.setState((s) => ({ results: { ...s.results, [project]: result } }));
      await reloadLoadedDirsUnder(result.dir);
      if (result.errors.length) console.warn(`generated sources of ${project}:`, result.errors.join("\n"));
    } catch (e) {
      console.warn(`generated sources: cannot compile ${project}`, e);
    } finally {
      useGenerated.setState((s) => {
        const running = new Set(s.running);
        running.delete(project);
        return { running };
      });
    }
  });
  return queue;
}

/** The project whose Analyzers node (or a folder below it) `path` is. */
function projectOfDir(path: string): string | null {
  for (const [project, dir] of Object.entries(get().dirs)) if (isSameOrInside(path, dir)) return project;
  return null;
}

/** Projects whose Analyzers node is expanded. */
function expandedProjects(expanded: ReadonlySet<string>): Set<string> {
  const out = new Set<string>();
  for (const k of expanded) {
    const p = projectOfDir(pathOfKey(k));
    if (p) out.add(p);
  }
  return out;
}

let timer: ReturnType<typeof setTimeout> | null = null;

/** Marks the projects containing `paths` stale; regenerates the expanded ones shortly. */
export function sourcesChanged(paths: readonly string[]) {
  const projects = Object.keys(get().dirs);
  let any = false;
  for (const path of paths) {
    if (!/\.(cs|vb|csproj|vbproj|props|targets|editorconfig|json|resx|razor|cshtml)$/i.test(path)) continue;
    if (Object.values(get().dirs).some((d) => isSameOrInside(path, d))) continue;
    for (const p of projects)
      if (isSameOrInside(path, parentOf(p)) || /Directory\.(Build|Packages)\.(props|targets)$/i.test(path)) {
        stale.add(p);
        any = true;
      }
  }
  if (!any) return;
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    for (const p of expandedProjects(useProjectTree.getState().expanded)) if (stale.has(p)) void generate(p);
  }, 1500);
}

/** Starts generating when an Analyzers node is expanded the first time (or after changes). */
export function watchGenerated(): () => void {
  let prev = new Set<string>();
  const check = (expanded: ReadonlySet<string>) => {
    const now = expandedProjects(expanded);
    for (const p of now) if (!prev.has(p) && (!get().results[p] || stale.has(p))) void generate(p);
    prev = now;
  };
  check(useProjectTree.getState().expanded);
  const unsub = useProjectTree.subscribe((s, old) => {
    if (s.expanded !== old.expanded) check(s.expanded);
  });
  return () => {
    unsub();
    if (timer) clearTimeout(timer);
    timer = null;
  };
}

export function resetGenerated() {
  epoch++;
  stale.clear();
  useGenerated.setState({ dirs: {}, results: {}, running: new Set() });
}
