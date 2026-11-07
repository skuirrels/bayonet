/**
 * Rider's Solution Explorer context menus: the solution node's, and the .NET items at the
 * top of a project node's menu (the Project view's own items follow them).
 */
import { commands } from "@/bindings";
import type { Solution } from "@/bindings";
import { executeAction, type Action } from "@/platform/actions";
import { call } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { copyText, type MenuItem } from "@/ui";
import { maybeEditorService, menuGroupItems, notifications } from "@/workbench/contributions";
import { revealLabel } from "@/features/project-tree/contextMenu";
import { revealablePath } from "@/features/project-tree/operations";
import { isEfProject } from "./ef/ef";
import type { SolutionProject } from "./projects/store";
import { useUnitTests } from "./tests/store";

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** Language servers that load the solution's projects. */
const PROJECT_SERVERS = new Set(["roslyn-language-server", "csharp-ls", "fsautocomplete"]);

const isTestProject = (project: SolutionProject) =>
  useUnitTests.getState().projects.some((p) => p.path === project.path);

function openFile(path: string) {
  void maybeEditorService()
    ?.openFile(path, { focus: true })
    .catch(() => undefined);
}

async function reveal(path: string) {
  try {
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    await revealItemInDir(revealablePath(path));
  } catch (e) {
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Cannot reveal ${fileName(path)}`,
      body: e instanceof Error ? e.message : String(e),
    });
  }
}

/** Rereads the solution and restarts the servers that loaded its projects (Rider's Reload All Projects). */
async function reloadProjects() {
  await executeAction("dotnet.reloadSolution");
  try {
    const servers = await call(commands.lspServers());
    await Promise.all(servers.filter((s) => PROJECT_SERVERS.has(s.name)).map((s) => call(commands.lspRestart(s.id))));
  } catch (e) {
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx("Cannot reload the projects"),
      body: e instanceof Error ? e.message : String(e),
    });
  }
}

function copyPathMenu(path: string): MenuItem {
  return {
    id: "copyPath",
    label: "Copy Path/Reference…",
    submenu: [
      { id: "abs", label: "Absolute Path", run: () => void copyText(path) },
      { id: "name", label: "File Name", run: () => void copyText(fileName(path)) },
    ],
  };
}

function openInMenu(path: string, dir: string): MenuItem {
  return {
    id: "openIn",
    label: "Open In",
    submenu: [
      { id: "finder", label: revealLabel(), icon: "external-link", run: () => void reveal(path) },
      { kind: "action", action: "terminal.openAt", arg: dir, label: "Terminal", icon: "terminal" },
      { kind: "separator" },
      ...menuGroupItems("openIn", { paths: [path], dirs: [false] }),
    ],
  };
}

/** The solution node's menu, in Rider's order. */
export function solutionMenuItems(solution: Solution): MenuItem[] {
  const dir = solution.path.slice(0, solution.path.lastIndexOf("/"));
  const add: MenuItem = {
    id: "add",
    label: "Add",
    icon: "plus",
    submenu: [
      {
        kind: "action",
        action: "dotnet.newProject",
        arg: { solutionFolder: null },
        label: "New Project…",
        icon: "module",
      },
      {
        kind: "action",
        action: "dotnet.addExistingProject",
        arg: { solutionFolder: null },
        label: "Existing Project…",
      },
    ],
  };
  return [
    add,
    { kind: "action", action: "dotnet.manageNugetPackages", label: "Manage NuGet Packages" },
    { kind: "action", action: "dotnet.reloadProjects" },
    ...(solution.format === "sln"
      ? [
          {
            id: "saveAs",
            label: "Save as…",
            submenu: [{ kind: "action" as const, action: "dotnet.migrateSolution", label: "Solution in .slnx Format" }],
          },
        ]
      : []),
    { kind: "separator" },
    { kind: "action", action: "build.buildProject", label: "Build Solution" },
    { kind: "separator" },
    { kind: "action", action: "dotnet.runAllTests", label: "Run Unit Tests" },
    { kind: "action", action: "dotnet.runAllTestsWithCoverage", label: "Cover Unit Tests" },
    {
      id: "advancedBuild",
      label: "Advanced Build Actions",
      submenu: [
        { kind: "action", action: "build.rebuildProject", label: "Rebuild Solution" },
        { kind: "action", action: "dotnet.showLastBinlog" },
        { kind: "separator" },
        { kind: "action", action: "build.stop", label: "Cancel Build" },
      ],
    },
    { kind: "separator" },
    { kind: "action", action: "vcs.localHistory", arg: dir, label: "Show Local History" },
    { kind: "separator" },
    { kind: "action", action: "analyze.inspectCode" },
    { kind: "action", action: "code.cleanup", label: "Reformat and Cleanup…" },
    { kind: "separator" },
    {
      id: "edit",
      label: "Edit",
      submenu: [
        { id: "editSolution", label: tx`Edit '${fileName(solution.path)}'`, run: () => openFile(solution.path) },
      ],
    },
    copyPathMenu(solution.path),
    openInMenu(solution.path, dir),
    { id: "diagrams", label: "Diagrams", submenu: [{ kind: "action", action: "dotnet.projectDiagram" }] },
    { kind: "separator" },
    { kind: "action", action: "dotnet.manageSdk" },
  ];
}

/** A solution folder's menu: what Rider offers for one besides the generic items. */
export function solutionFolderMenuItems(folder: string): MenuItem[] {
  const arg = { solutionFolder: folder };
  return [
    {
      id: "add",
      label: "Add",
      icon: "plus",
      submenu: [
        { kind: "action", action: "dotnet.newProject", arg, label: "New Project…", icon: "module" },
        { kind: "action", action: "dotnet.addExistingProject", arg, label: "Existing Project…" },
      ],
    },
    { kind: "separator" },
    { kind: "action", action: "build.buildProject", label: "Build Solution" },
    { kind: "action", action: "dotnet.runAllTests", label: "Run Unit Tests" },
    { kind: "action", action: "dotnet.manageNugetPackages", label: "Manage NuGet Packages" },
  ];
}

/**
 * The .NET items Rider shows first for a project: Add › (with the Project view's New items),
 * NuGet, Edit, EF Core, then build, tests and publishing.
 */
export function projectMenuItems(project: SolutionProject, newItems: MenuItem[] = []): MenuItem[] {
  const arg = { dir: project.dir };
  const items: Array<MenuItem | false> = [
    {
      id: "new",
      label: "Add",
      icon: "plus",
      submenu: [
        ...newItems,
        { kind: "separator" },
        { kind: "action", action: "dotnet.projectReferences", arg, label: "Project Reference…" },
        { kind: "action", action: "dotnet.manageNugetPackages", label: "NuGet Package…" },
      ],
    },
    { kind: "action", action: "dotnet.manageNugetPackages", label: "Manage NuGet Packages" },
    {
      id: "edit",
      label: "Edit",
      submenu: [{ id: "editProject", label: tx`Edit '${fileName(project.path)}'`, run: () => openFile(project.path) }],
    },
    isEfProject(project) && {
      id: "ef",
      label: "Entity Framework Core",
      icon: "database",
      submenu: [
        { kind: "action", action: "dotnet.ef.addMigration", arg },
        { kind: "action", action: "dotnet.ef.removeMigration", arg },
        { kind: "action", action: "dotnet.ef.updateDatabase", arg },
        { kind: "separator" },
        { kind: "action", action: "dotnet.ef.script", arg },
        { kind: "action", action: "dotnet.ef.scaffold", arg },
      ],
    },
    { kind: "action", action: "dotnet.manageUserSecrets", arg },
    { kind: "separator" },
    { kind: "action", action: "build.buildModule", arg: project.dir, label: "Build Selected Projects" },
    { kind: "action", action: "build.rebuildProject", arg: project.dir, label: "Rebuild Selected Projects" },
    isTestProject(project) && { kind: "action", action: "dotnet.runProjectTests", arg },
    isTestProject(project) && { kind: "action", action: "dotnet.coverProjectTests", arg },
    { kind: "action", action: "dotnet.publish", arg },
    { kind: "action", action: "dotnet.runInContainer", arg },
    { kind: "separator" },
    { id: "diagrams", label: "Diagrams", submenu: [{ kind: "action", action: "dotnet.projectDiagram" }] },
    { kind: "action", action: "dotnet.removeFromSolution", arg, label: "Remove from Solution" },
  ];
  return items.filter((i): i is MenuItem => !!i);
}

/** Actions the menus add: Reload All Projects, Manage .NET SDK, a project's tests. */
export function contextMenuActions(
  runTests: (project: SolutionProject, coverage: boolean) => void,
  projectOf: (arg: unknown) => SolutionProject | null,
): Action[] {
  return [
    {
      id: "dotnet.reloadProjects",
      title: "Reload All Projects",
      description: "Rereads the solution and restarts the C# and F# language servers, which load its projects again",
      category: "Project",
      icon: "refresh",
      when: "projectOpen",
      keywords: ["reload", "projects", "solution", "roslyn", "restart"],
      run: () => void reloadProjects(),
    },
    {
      id: "dotnet.manageSdk",
      title: "Manage .NET SDK…",
      description: "Opens Settings › Languages › .NET: the dotnet host, SDK and global.json",
      category: "Project",
      when: "projectOpen",
      keywords: ["sdk", "dotnet", "global.json", "toolset", "msbuild"],
      run: () => void executeAction("file.settings", "languages.dotnet"),
    },
    {
      id: "dotnet.runProjectTests",
      title: "Run Unit Tests",
      category: "Unit Tests",
      icon: "gutter-run-all",
      hidden: true,
      when: "projectOpen",
      run: ({ arg }) => {
        const project = projectOf(arg);
        if (project) runTests(project, false);
      },
    },
    {
      id: "dotnet.coverProjectTests",
      title: "Cover Unit Tests",
      category: "Unit Tests",
      icon: "run-coverage",
      hidden: true,
      when: "projectOpen",
      run: ({ arg }) => {
        const project = projectOf(arg);
        if (project) runTests(project, true);
      },
    },
  ];
}
