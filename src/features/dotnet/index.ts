/**
 * .NET: the Solution pane of the Project view, from the `.slnx` / `.sln` /
 * `.slnf` at the project root (read by the backend, no MSBuild evaluation).
 */
import { commands } from "@/bindings";
import { executeAction, registerActions } from "@/platform/actions";
import { call, isTauri, onIdeEvent } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import {
  maybeEditorService,
  notifications,
  registerEditorKind,
  registerFileEditorProvider,
  registerMenuGroup,
  registerOverlay,
  registerProjectViewRoot,
  registerRunConfigType,
  registerStatusItem,
  registerToolWindow,
} from "@/workbench/contributions";
import { activateToolWindow } from "@/workbench/toolwindows/store";
import { lazyComponent } from "@/ui";
import { registerEditorContribution } from "@/editor/api";
import { loadInstalled, resetNuget } from "./nuget/store";
import { testGutter } from "./tests/gutter";
import { msbuildNavigation } from "./msbuild/navigation";
import { SolutionDialogs } from "./projects/SolutionDialogs";
import {
  addExistingProject,
  migrateSolution,
  openDialog,
  projectAt,
  removeFromSolution,
  setSolution,
  useSolution,
} from "./projects/store";
import {
  discoverTests,
  loadProjects,
  resetUnitTests,
  runTests,
  runTestsWithCoverage,
  useUnitTests,
} from "./tests/store";
import { FrameworkStatus } from "./FrameworkStatus";
import { GROUP_PREFIX } from "@/features/project-tree/model";
import { setExpanded, suggestViewMode, useProjectTree } from "@/features/project-tree/store";
import { dotnetRunType } from "./runType";
import { efActions, isEfProject, loadEfProjects } from "./ef/ef";
import { msbuildCompletion } from "./msbuild/completion";
import { SOLUTION_ROOT_ID, solutionProjects, solutionRoot, touchesSolution } from "./solution";
import { loadGeneratedDirs, resetGenerated, sourcesChanged, watchGenerated } from "./generated";
import { tx } from "@/platform/i18n";

const BINLOG_KIND = "binlog";
const isBinlog = (path: string) => /\.binlog$/i.test(path);
const basenameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** Opens the newest binary log of this project's builds (Settings › .NET › Build). */
async function showLastBinlog() {
  const [last] = await call(commands.dotnetBinlogs()).catch(() => [] as string[]);
  if (!last) {
    notifications().notify({
      group: "dotnet",
      level: "info",
      title: tx`No binary log yet`,
      body: tx`Turn on "Write a binary log of every build" in Settings › Languages › .NET, then build.`,
      actions: [{ label: tx`Open Settings`, run: () => void executeAction("file.settings", "languages.dotnet") }],
    });
    return;
  }
  await maybeEditorService()
    ?.openFile(last, { focus: true })
    .catch(() => undefined);
}

const REFRESH_DELAY = 300;
export const NUGET_WINDOW = "nuget";
export const UNIT_TESTS_WINDOW = "unitTests";
export const DIAGNOSTICS_WINDOW = "dotnetDiagnostics";

const diagnostics = () => import("./diagnostics/store");

/** Shows the Diagnostics window on `tab` and runs `then` with its store. */
function withDiagnostics(
  tab: import("./diagnostics/store").DiagTab,
  then?: (m: typeof import("./diagnostics/store")) => unknown,
) {
  activateToolWindow(DIAGNOSTICS_WINDOW);
  void diagnostics().then(async (m) => {
    m.setTab(tab);
    if (!then) return;
    if (m.useDiagnostics.getState().pid == null) await m.loadProcesses();
    await then(m);
  });
}
/** Files whose change can change the installed packages. */
const PACKAGE_FILE = /(?:\.(?:cs|fs|vb)proj|(?:^|\/)Directory\.(?:Packages|Build)\.props)$/i;

export function activate(): () => void {
  let disposeRoot = () => {};
  let disposeNuget: (() => void) | null = null;
  let disposeTests: (() => void) | null = null;
  let disposeDiagnostics: (() => void) | null = null;
  let seq = 0;
  let shown = "";
  const clear = () => {
    disposeRoot();
    disposeRoot = () => {};
    setSolution(null);
    disposeNuget?.();
    disposeNuget = null;
    disposeTests?.();
    disposeTests = null;
    if (disposeDiagnostics) void diagnostics().then((m) => m.resetDiagnostics());
    disposeDiagnostics?.();
    disposeDiagnostics = null;
    resetNuget();
    resetUnitTests();
    resetGenerated();
    shown = "";
  };
  const refresh = async () => {
    const id = ++seq;
    if (!useProject.getState().project || !isTauri()) return clear();
    try {
      const [path] = await call(commands.dotnetSolutions());
      if (id !== seq) return;
      if (!path) return clear();
      const solution = await call(commands.dotnetSolution(path));
      if (id !== seq) return;
      const dirs = await loadGeneratedDirs(solutionProjects(solution.items)).catch((e: unknown) => {
        console.warn("solution: no folders for generated sources", e);
        return {};
      });
      if (id !== seq) return;
      const key = JSON.stringify([solution, dirs]);
      if (key === shown) return;
      const first = !shown;
      disposeRoot();
      shown = key;
      disposeRoot = registerProjectViewRoot(solutionRoot(solution, dirs));
      setSolution(solution);
      void loadEfProjects();
      // Kept across solution edits so the window keeps its search and selection.
      disposeNuget ??= registerToolWindow({
        id: NUGET_WINDOW,
        title: "NuGet",
        icon: "toolwindow-nuget",
        anchor: "bottom",
        order: 40,
        component: lazyComponent(() => import("./nuget/NugetView").then((m) => m.NugetView)),
      });
      disposeDiagnostics ??= registerToolWindow({
        id: DIAGNOSTICS_WINDOW,
        title: ".NET Diagnostics",
        icon: "toolwindow-profiler",
        anchor: "bottom",
        order: 42,
        component: lazyComponent(() => import("./diagnostics/DiagnosticsView").then((m) => m.DiagnosticsView)),
      });
      if (!disposeTests) {
        disposeTests = registerToolWindow({
          id: UNIT_TESTS_WINDOW,
          title: "Unit Tests",
          icon: "toolwindow-unit-tests",
          anchor: "bottom",
          order: 41,
          component: lazyComponent(() => import("./tests/UnitTestsView").then((m) => m.UnitTestsView)),
        });
        void loadProjects();
      }
      if (first) {
        suggestViewMode("solution");
        setExpanded(new Set([...useProjectTree.getState().expanded, `${GROUP_PREFIX}${SOLUTION_ROOT_ID}`]));
      }
    } catch (e) {
      if (id === seq) console.warn("solution: cannot read the solution", e);
    }
  };
  void refresh();
  const disposeRunType = registerRunConfigType(dotnetRunType);
  const disposeBinlogKind = registerEditorKind({
    kind: BINLOG_KIND,
    component: lazyComponent(() => import("./binlog/BinlogViewer").then((m) => m.BinlogViewer)),
    title: (input) => basenameOf(input.path ?? input.key),
    icon: () => "toolwindow-build",
    restorable: true,
  });
  const disposeBinlogProvider = registerFileEditorProvider({
    id: "dotnet.binlog",
    order: 5,
    resolve: (path) => (isBinlog(path) ? { kind: BINLOG_KIND, key: `binlog:${path}`, path } : null),
  });
  const disposeStatus = registerStatusItem({
    id: "dotnet.targetFramework",
    title: "Target Framework",
    align: "right",
    priority: 6,
    component: FrameworkStatus,
  });
  const disposeGutter = registerEditorContribution({
    id: "dotnet.tests",
    order: 56,
    extensions: testGutter,
  });
  const disposeMsbuild = registerEditorContribution({
    id: "dotnet.msbuild",
    order: 57,
    extensions: (ctx) => [msbuildNavigation(ctx), msbuildCompletion(ctx)],
  });
  const disposeDialogs = registerOverlay({
    id: "dotnet.solutionDialogs",
    component: SolutionDialogs,
  });
  const disposeNewMenu = registerMenuGroup({
    id: "dotnet.new",
    menus: ["new"],
    order: 5,
    items: (_menu, { paths, dirs }) => {
      if (!useSolution.getState().solution) return [];
      const dir = paths.length === 1 && dirs?.[0] ? paths[0]! : null;
      return [
        {
          kind: "action",
          action: "dotnet.newProject",
          arg: { directory: dir },
          label: "Project…",
          icon: "module",
        },
      ];
    },
  });
  const solutionKey = `${GROUP_PREFIX}${SOLUTION_ROOT_ID}`;
  const disposeSolutionMenu = registerMenuGroup({
    id: "dotnet.solution",
    menus: ["projectViewGroup"],
    order: 10,
    items: (_menu, { paths }) => {
      const key = paths.length === 1 ? paths[0]! : null;
      if (!key || !useSolution.getState().solution) return [];
      // The solution node, or one of its solution folders (`<root key>/src/inner`).
      const folder =
        key === solutionKey ? null : key.startsWith(`${solutionKey}/`) ? key.slice(solutionKey.length + 1) : undefined;
      if (folder === undefined) return [];
      const arg = { solutionFolder: folder };
      return [
        {
          label: "Add",
          icon: "plus",
          submenu: [
            {
              kind: "action",
              action: "dotnet.newProject",
              arg,
              label: "New Project…",
              icon: "module",
            },
            {
              kind: "action",
              action: "dotnet.addExistingProject",
              arg,
              label: "Existing Project…",
            },
          ],
        },
        { kind: "separator" },
        ...(folder === null && useSolution.getState().solution?.format === "sln"
          ? [{ kind: "action" as const, action: "dotnet.migrateSolution" }]
          : []),
        { kind: "action", action: "dotnet.runAllTests" },
        { kind: "action", action: "dotnet.manageNugetPackages" },
      ];
    },
  });
  const disposeProjectMenu = registerMenuGroup({
    id: "dotnet.project",
    menus: ["projectView"],
    order: 25,
    items: (_menu, { paths, dirs }) => {
      const project = paths.length === 1 && dirs?.[0] ? projectAt(paths[0]!) : null;
      if (!project) return [];
      return [
        {
          kind: "action",
          action: "dotnet.projectReferences",
          arg: { dir: project.dir },
          label: "Add Project Reference…",
        },
        {
          kind: "action",
          action: "dotnet.manageNugetPackages",
          label: "Manage NuGet Packages…",
        },
        ...(isEfProject(project)
          ? [
              {
                label: "Entity Framework Core",
                icon: "database",
                submenu: [
                  { kind: "action" as const, action: "dotnet.ef.addMigration", arg: { dir: project.dir } },
                  { kind: "action" as const, action: "dotnet.ef.removeMigration", arg: { dir: project.dir } },
                  { kind: "action" as const, action: "dotnet.ef.updateDatabase", arg: { dir: project.dir } },
                  { kind: "separator" as const },
                  { kind: "action" as const, action: "dotnet.ef.script", arg: { dir: project.dir } },
                  { kind: "action" as const, action: "dotnet.ef.scaffold", arg: { dir: project.dir } },
                ],
              },
            ]
          : []),
        {
          kind: "action",
          action: "dotnet.removeFromSolution",
          arg: { dir: project.dir },
          label: "Remove from Solution",
        },
      ];
    },
  });
  const dirArg = (arg: unknown, key: string) => {
    const v = (arg ?? {}) as Record<string, unknown>;
    return typeof v[key] === "string" ? (v[key] as string) : null;
  };
  const showTests = () => {
    if (disposeTests) activateToolWindow(UNIT_TESTS_WINDOW);
    return !!disposeTests;
  };
  const disposeActions = registerActions([
    {
      id: "dotnet.showLastBinlog",
      title: "Show Last Binary Log",
      description: "Opens the MSBuild binary log of the last .NET build in the binary log viewer",
      category: "Build",
      icon: "toolwindow-build",
      when: "projectOpen",
      keywords: ["binlog", "binary log", "msbuild", "structured log", "build log"],
      run: () => void showLastBinlog(),
    },
    {
      id: "dotnet.newProject",
      title: "New Project…",
      description: "Creates a project from an installed dotnet new template and adds it to the solution",
      category: "Project",
      icon: "module",
      when: "projectOpen",
      keywords: ["new", "project", "template", "dotnet new", "add project"],
      run: ({ arg }) =>
        openDialog({
          kind: "newProject",
          directory: dirArg(arg, "directory"),
          solutionFolder: dirArg(arg, "solutionFolder"),
        }),
    },
    {
      id: "dotnet.migrateSolution",
      title: "Convert Solution to .slnx",
      description: "Writes a .slnx next to the .sln with dotnet sln migrate; the .sln is kept",
      category: "Project",
      when: "projectOpen",
      keywords: ["slnx", "sln", "migrate", "convert", "solution"],
      run: () => void migrateSolution(),
    },
    {
      id: "dotnet.addExistingProject",
      title: "Add Existing Project…",
      category: "Project",
      when: "projectOpen",
      keywords: ["add", "project", "solution", "existing"],
      run: ({ arg }) => void addExistingProject(dirArg(arg, "solutionFolder")),
    },
    {
      id: "dotnet.projectReferences",
      title: "Add Project Reference…",
      category: "Project",
      hidden: true,
      when: "projectOpen",
      run: ({ arg }) => {
        const project = projectAt(dirArg(arg, "dir") ?? "");
        if (project) openDialog({ kind: "references", project });
      },
    },
    {
      id: "dotnet.removeFromSolution",
      title: "Remove from Solution",
      category: "Project",
      hidden: true,
      when: "projectOpen",
      run: ({ arg }) => {
        const project = projectAt(dirArg(arg, "dir") ?? "");
        if (project) void removeFromSolution([project]);
      },
    },
    {
      id: "dotnet.runAllTests",
      title: "Run All Tests in Solution",
      category: "Unit Tests",
      icon: "gutter-run-all",
      when: "projectOpen",
      keywords: ["test", "tests", "unit", "xunit", "nunit", "mstest"],
      run: () => {
        if (showTests() && !useUnitTests.getState().busy) void runTests();
      },
    },
    {
      id: "dotnet.runAllTestsWithCoverage",
      title: "Run All Tests in Solution with Coverage",
      category: "Unit Tests",
      icon: "run-coverage",
      when: "projectOpen",
      keywords: ["test", "tests", "unit", "coverage", "cover"],
      run: () => {
        if (showTests() && !useUnitTests.getState().busy) void runTestsWithCoverage();
      },
    },
    {
      id: "dotnet.discoverTests",
      title: "Build and Discover Tests",
      category: "Unit Tests",
      icon: "refresh",
      when: "projectOpen",
      keywords: ["test", "tests", "unit", "explorer"],
      run: () => {
        if (showTests() && !useUnitTests.getState().busy) void discoverTests();
      },
    },
    ...efActions(),
    {
      id: "dotnet.reloadSolution",
      title: "Reload Solution",
      category: "Project",
      when: "projectOpen",
      run: refresh,
    },
    {
      id: "dotnet.diagnostics",
      title: ".NET Diagnostics",
      description: "Live counters, thread stacks, GC heap snapshots, dumps and CPU traces of a running .NET process",
      category: "Diagnostics",
      icon: "toolwindow-profiler",
      when: "projectOpen",
      keywords: ["diagnostics", "counters", "dotnet-counters", "dotnet-monitor", "profiler", "metrics", "performance"],
      run: () => withDiagnostics("live"),
    },
    {
      id: "dotnet.captureStacks",
      title: "Capture Thread Stacks",
      description: "Captures the managed stack of every thread of the selected .NET process (dotnet-stack)",
      category: "Diagnostics",
      icon: "hierarchy",
      when: "projectOpen",
      keywords: ["stacks", "threads", "dotnet-stack", "hang", "deadlock"],
      run: () => withDiagnostics("threads", (m) => m.collectStacks()),
    },
    {
      id: "dotnet.captureGcHeap",
      title: "Capture GC Heap Snapshot",
      description: "Snapshots the GC heap of the selected .NET process by type (dotnet-gcdump)",
      category: "Diagnostics",
      icon: "package",
      when: "projectOpen",
      keywords: ["gcdump", "heap", "memory", "leak", "dotnet-gcdump", "snapshot"],
      run: () => withDiagnostics("memory", (m) => m.collectGcdump()),
    },
    {
      id: "dotnet.collectDump",
      title: "Collect Process Dump",
      description: "Writes a dump of the selected .NET process and analyses it with SOS (dotnet-dump)",
      category: "Diagnostics",
      icon: "debug",
      when: "projectOpen",
      keywords: ["dump", "dotnet-dump", "sos", "clrstack", "dumpheap", "crash"],
      run: () => withDiagnostics("dumps", (m) => m.collectDump()),
    },
    {
      id: "dotnet.recordCpuTrace",
      title: "Record CPU Trace",
      description: "Samples the selected .NET process's stacks and shows a flame graph (dotnet-trace)",
      category: "Diagnostics",
      icon: "profile",
      when: "projectOpen",
      keywords: ["trace", "dotnet-trace", "cpu", "profile", "flame graph", "sampling"],
      run: () => withDiagnostics("trace", (m) => m.collectTrace()),
    },
    {
      id: "dotnet.sosShell",
      title: "Open SOS Shell on Process",
      description: "Attaches dotnet-debug to the selected .NET process and opens its SOS shell in the terminal",
      category: "Diagnostics",
      icon: "attach-to-process",
      when: "projectOpen",
      keywords: ["sos", "dotnet-debug", "attach", "clrstack", "debugger extension"],
      run: () => withDiagnostics("threads", (m) => m.attachSosShell()),
    },
    {
      id: "dotnet.manageNugetPackages",
      title: "Manage NuGet Packages…",
      description: "Opens the NuGet tool window: search, install, update and remove packages",
      category: "NuGet",
      icon: "toolwindow-nuget",
      when: "projectOpen",
      keywords: ["nuget", "package", "packages", "dependency", "install", "update"],
      run: () => {
        if (disposeNuget) activateToolWindow(NUGET_WINDOW);
      },
    },
  ]);
  const unsub = useProject.subscribe((s, prev) => {
    if (s.project?.root === prev.project?.root) return;
    resetNuget();
    resetUnitTests();
    void refresh().then(() => {
      if (disposeNuget) void loadInstalled();
      if (disposeTests) void loadProjects();
    });
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  let packagesTimer: ReturnType<typeof setTimeout> | undefined;
  const unwatchGenerated = watchGenerated();
  const unsubFs = onIdeEvent("fsChanged", (e) => {
    const paths = [...e.created, ...e.modified, ...e.removed];
    sourcesChanged(paths);
    if (disposeNuget && paths.some((p) => PACKAGE_FILE.test(p))) {
      clearTimeout(packagesTimer);
      packagesTimer = setTimeout(() => void loadInstalled(), REFRESH_DELAY * 3);
    }
    if (!touchesSolution(paths)) return;
    clearTimeout(timer);
    timer = setTimeout(() => void refresh(), REFRESH_DELAY);
  });
  return () => {
    seq++;
    clearTimeout(timer);
    clearTimeout(packagesTimer);
    unsub();
    unsubFs();
    unwatchGenerated();
    disposeActions();
    disposeGutter();
    disposeMsbuild();
    disposeDialogs();
    disposeNewMenu();
    disposeProjectMenu();
    disposeSolutionMenu();
    disposeRunType();
    disposeStatus();
    disposeBinlogProvider();
    disposeBinlogKind();
    clear();
  };
}
