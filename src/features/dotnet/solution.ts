/**
 * The Solution pane's tree for a .NET solution: solution folders as groups,
 * projects as their folders (with Dependencies first: Analyzers for C# and VB,
 * see `generated.ts`, then Frameworks, Packages and Projects, see
 * `dependencies.ts`), solution items as files.
 */
import type { ProjectLanguage, Solution, SolutionItem } from "@/bindings";
import type { ProjectViewRootContribution, ProjectViewRootEntry } from "@/workbench/contributions";
import { nameOf, parentOf } from "@/features/project-tree/paths";
import { tx } from "@/platform/i18n";
import { dependencyEntries, type DependencyMap } from "./dependencies";

export const SOLUTION_ROOT_ID = "solution";

const PROJECT_ICONS: Partial<Record<ProjectLanguage, string>> = {
  csharp: "project-csharp",
  fsharp: "project-fsharp",
  sql: "file-sql",
};

/** Files whose change can change the solution tree. */
const SOLUTION_FILE = /\.(slnx?|slnf|csproj|fsproj|vbproj|sqlproj)$/i;

export const touchesSolution = (paths: readonly string[]) => paths.some((p) => SOLUTION_FILE.test(p));

function projectCount(items: readonly SolutionItem[]): number {
  return items.reduce((n, i) => n + (i.kind === "project" ? 1 : i.kind === "folder" ? projectCount(i.items) : 0), 0);
}

/** Cache folder of each project's generated sources, by project file. */
export type GeneratedDirs = Readonly<Record<string, string>>;

function dependencies(
  generated: string | undefined,
  deps: DependencyMap[string] | undefined,
): ProjectViewRootEntry[] | undefined {
  const children: ProjectViewRootEntry[] = [
    ...(generated ? [{ label: tx("Analyzers"), icon: "analyzer", path: generated, readOnly: true }] : []),
    ...dependencyEntries(deps),
  ];
  if (!children.length) return undefined;
  return [{ id: "dependencies", label: tx("Dependencies"), icon: "library", children }];
}

function entry(item: SolutionItem, dirs: GeneratedDirs, deps: DependencyMap): ProjectViewRootEntry {
  switch (item.kind) {
    case "folder":
      return {
        label: item.name,
        icon: "folder",
        children: item.items.map((i) => entry(i, dirs, deps)),
      };
    case "project":
      return {
        label: item.name,
        icon: PROJECT_ICONS[item.language] ?? "module",
        path: parentOf(item.path),
        // Build output and the project file, as Visual Studio and Rider leave them out of the
        // Solution Explorer; the project's Edit menu opens the project file.
        hide: ["bin", "obj", nameOf(item.path)],
        detail: item.missing ? tx("not found") : undefined,
        prepend: item.missing ? undefined : dependencies(dirs[item.path], deps[item.path]),
      };
    case "file":
      return {
        label: item.path.slice(item.path.lastIndexOf("/") + 1),
        icon: "",
        path: item.path,
        file: true,
      };
  }
}

/** Project files of the solution, depth first. */
export function solutionProjects(items: readonly SolutionItem[]): string[] {
  return items.flatMap((i) => (i.kind === "project" ? [i.path] : i.kind === "folder" ? solutionProjects(i.items) : []));
}

/** `Solution 'Name'` with its projects below, as Rider and Visual Studio show it. */
export function solutionRoot(
  solution: Solution,
  dirs: GeneratedDirs = {},
  deps: DependencyMap = {},
): ProjectViewRootContribution {
  const n = projectCount(solution.items);
  return {
    id: SOLUTION_ROOT_ID,
    label: tx`Solution '${solution.name}'`,
    icon: "module",
    detail: n === 1 ? tx("1 project") : tx`${n} projects`,
    scope: "solution",
    children: solution.items.map((i) => entry(i, dirs, deps)),
  };
}
