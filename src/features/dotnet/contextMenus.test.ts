import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Solution } from "@/bindings";
import { registerActions } from "@/platform/actions";
import { useProject } from "@/platform/project";
import { resolveMenuItems, type MenuItem } from "@/ui";
import { projectMenuItems, solutionMenuItems } from "./contextMenus";
import { useSolution } from "./projects/store";
import { useUnitTests } from "./tests/store";

/** Labels as the menu shows them ("-" for separators). */
const labels = (items: MenuItem[]) => resolveMenuItems(items).map((r) => (r.kind === "separator" ? "-" : r.label));
const sub = (items: MenuItem[], label: string) => {
  const item = resolveMenuItems(items).find((r) => r.label === label);
  return item?.submenu ? labels(item.submenu) : undefined;
};

const ACTIONS = [
  "dotnet.newProject",
  "dotnet.addExistingProject",
  "dotnet.manageNugetPackages",
  "dotnet.reloadProjects",
  "dotnet.migrateSolution",
  "build.buildProject",
  "build.buildModule",
  "build.rebuildProject",
  "build.stop",
  "dotnet.runAllTests",
  "dotnet.runAllTestsWithCoverage",
  "dotnet.showLastBinlog",
  "vcs.localHistory",
  "analyze.inspectCode",
  "code.cleanup",
  "terminal.openAt",
  "dotnet.projectDiagram",
  "dotnet.manageSdk",
  "dotnet.projectReferences",
  "dotnet.manageUserSecrets",
  "dotnet.runProjectTests",
  "dotnet.coverProjectTests",
  "dotnet.publish",
  "dotnet.runInContainer",
  "dotnet.removeFromSolution",
];

const solution = (format: Solution["format"]): Solution => ({
  path: `/s/App.${format}`,
  name: "App",
  format,
  items: [],
});
const app = { name: "App", path: "/s/src/App/App.csproj", dir: "/s/src/App", folders: [] };
const tests = {
  name: "App.Tests",
  path: "/s/tests/App.Tests/App.Tests.csproj",
  dir: "/s/tests/App.Tests",
  folders: [],
};

let dispose = () => {};

beforeEach(() => {
  useProject.setState({ project: { name: "s", root: "/s", trusted: true, isGit: false, settings: {} } as never });
  dispose = registerActions(ACTIONS.map((id) => ({ id, title: id, run: () => undefined })));
});

afterEach(() => {
  dispose();
  useProject.setState({ project: null });
  useSolution.setState({ efProjects: [] });
  useUnitTests.setState({ projects: [] });
});

describe("Solution Explorer context menus", () => {
  it("follows Rider's order for the solution", () => {
    const items = solutionMenuItems(solution("slnx"));
    expect(labels(items)).toEqual([
      "Add",
      "Manage NuGet Packages",
      "dotnet.reloadProjects",
      "-",
      "Build Solution",
      "-",
      "Run Unit Tests",
      "Cover Unit Tests",
      "Advanced Build Actions",
      "-",
      "Show Local History",
      "-",
      "analyze.inspectCode",
      "Reformat and Cleanup…",
      "-",
      "Edit",
      "Copy Path/Reference…",
      "Open In",
      "Diagrams",
      "-",
      "dotnet.manageSdk",
    ]);
    expect(sub(items, "Add")).toEqual(["New Project…", "Existing Project…"]);
    expect(sub(items, "Edit")).toEqual(["Edit 'App.slnx'"]);
    expect(sub(items, "Advanced Build Actions")).toEqual([
      "Rebuild Solution",
      "dotnet.showLastBinlog",
      "-",
      "Cancel Build",
    ]);
  });

  it("offers Save as .slnx for a .sln only", () => {
    expect(labels(solutionMenuItems(solution("slnx")))).not.toContain("Save as…");
    expect(sub(solutionMenuItems(solution("sln")), "Save as…")).toEqual(["Solution in .slnx Format"]);
  });

  it("leads a project's menu with Add, NuGet, Edit and its builds", () => {
    const items = projectMenuItems(app, [{ id: "new.file", label: "File" }]);
    expect(labels(items)).toEqual([
      "Add",
      "Manage NuGet Packages",
      "Edit",
      "dotnet.manageUserSecrets",
      "-",
      "Build Selected Projects",
      "Rebuild Selected Projects",
      "dotnet.publish",
      "dotnet.runInContainer",
      "-",
      "Diagrams",
      "Remove from Solution",
    ]);
    expect(sub(items, "Add")).toEqual(["File", "-", "Project Reference…", "NuGet Package…"]);
    expect(sub(items, "Edit")).toEqual(["Edit 'App.csproj'"]);
    const build = items.find((i) => i.kind === "action" && i.action === "build.rebuildProject");
    expect(build).toMatchObject({ arg: "/s/src/App" });
  });

  it("offers a test project's tests and an EF Core project's migrations", () => {
    useUnitTests.setState({
      projects: [{ path: tests.path, name: tests.name, tests: new Map(), message: null, busy: false }] as never,
    });
    useSolution.setState({ efProjects: [tests.path] });
    const l = labels(projectMenuItems(tests));
    expect(l).toContain("dotnet.runProjectTests");
    expect(l).toContain("dotnet.coverProjectTests");
    expect(l).toContain("Entity Framework Core");
    expect(labels(projectMenuItems(app))).not.toContain("dotnet.runProjectTests");
  });
});
