/**
 * .NET: the Solution pane of the Project view, from the `.slnx` / `.sln` /
 * `.slnf` at the project root (read by the backend, no MSBuild evaluation).
 */
import { commands } from "@/bindings";
import { registerActions } from "@/platform/actions";
import { call, isTauri, onIdeEvent } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import {
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
  openDialog,
  projectAt,
  removeFromSolution,
  setSolution,
  useSolution,
} from "./projects/store";
import { discoverTests, loadProjects, resetUnitTests, runTests, useUnitTests } from "./tests/store";
import { FrameworkStatus } from "./FrameworkStatus";
import { GROUP_PREFIX } from "@/features/project-tree/model";
import { setExpanded, suggestViewMode, useProjectTree } from "@/features/project-tree/store";
import { dotnetRunType } from "./runType";
import { SOLUTION_ROOT_ID, solutionRoot, touchesSolution } from "./solution";

const REFRESH_DELAY = 300;
export const NUGET_WINDOW = "nuget";
export const UNIT_TESTS_WINDOW = "unitTests";
/** Files whose change can change the installed packages. */
const PACKAGE_FILE = /(?:\.(?:cs|fs|vb)proj|(?:^|\/)Directory\.(?:Packages|Build)\.props)$/i;

export function activate(): () => void {
  let disposeRoot = () => {};
  let disposeNuget: (() => void) | null = null;
  let disposeTests: (() => void) | null = null;
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
    resetNuget();
    resetUnitTests();
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
      const key = JSON.stringify(solution);
      if (key === shown) return;
      const first = !shown;
      disposeRoot();
      shown = key;
      disposeRoot = registerProjectViewRoot(solutionRoot(solution));
      setSolution(solution);
      // Kept across solution edits so the window keeps its search and selection.
      disposeNuget ??= registerToolWindow({
        id: NUGET_WINDOW,
        title: "NuGet",
        icon: "toolwindow-nuget",
        anchor: "bottom",
        order: 40,
        component: lazyComponent(() => import("./nuget/NugetView").then((m) => m.NugetView)),
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
    extensions: msbuildNavigation,
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
    {
      id: "dotnet.reloadSolution",
      title: "Reload Solution",
      category: "Project",
      when: "projectOpen",
      run: refresh,
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
  const unsubFs = onIdeEvent("fsChanged", (e) => {
    const paths = [...e.created, ...e.modified, ...e.removed];
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
    disposeActions();
    disposeGutter();
    disposeMsbuild();
    disposeDialogs();
    disposeNewMenu();
    disposeProjectMenu();
    disposeSolutionMenu();
    disposeRunType();
    disposeStatus();
    clear();
  };
}
