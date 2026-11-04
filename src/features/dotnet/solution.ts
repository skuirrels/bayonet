/**
 * The Solution pane's tree for a .NET solution: solution folders as groups,
 * projects as their folders, solution items as files.
 */
import type { ProjectLanguage, Solution, SolutionItem } from "@/bindings";
import type { ProjectViewRootContribution, ProjectViewRootEntry } from "@/workbench/contributions";
import { parentOf } from "@/features/project-tree/paths";
import { tx } from "@/platform/i18n";

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

function entry(item: SolutionItem): ProjectViewRootEntry {
  switch (item.kind) {
    case "folder":
      return {
        label: item.name,
        icon: "folder",
        children: item.items.map(entry),
      };
    case "project":
      return {
        label: item.name,
        icon: PROJECT_ICONS[item.language] ?? "module",
        path: parentOf(item.path),
        // Build output, as Visual Studio and Rider leave it out of the Solution Explorer.
        hide: ["bin", "obj"],
        detail: item.missing ? tx("not found") : undefined,
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

/** `Solution 'Name'` with its projects below, as Rider and Visual Studio show it. */
export function solutionRoot(solution: Solution): ProjectViewRootContribution {
  const n = projectCount(solution.items);
  return {
    id: SOLUTION_ROOT_ID,
    label: tx`Solution '${solution.name}'`,
    icon: "module",
    detail: n === 1 ? tx("1 project") : tx`${n} projects`,
    scope: "solution",
    children: solution.items.map(entry),
  };
}
