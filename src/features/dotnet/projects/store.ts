/** Solution edits: the open solution's projects, and which solution dialog is showing. */
import { create } from "zustand";
import { commands } from "@/bindings";
import type { Solution, SolutionItem } from "@/bindings";
import { call } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { notifications } from "@/workbench/contributions";

export interface SolutionProject {
  name: string;
  /** Project file. */
  path: string;
  /** Its folder (what the Solution Explorer node stands for). */
  dir: string;
  /** Solution folders it sits in, outermost first. */
  folders: string[];
}

export type SolutionDialog =
  | {
      kind: "newProject";
      directory: string | null;
      solutionFolder?: string | null;
    }
  | { kind: "references"; project: SolutionProject };

interface SolutionState {
  solution: Solution | null;
  projects: SolutionProject[];
  /** Project files of the projects that use Entity Framework Core. */
  efProjects: string[];
  dialog: SolutionDialog | null;
}

export const useSolution = create<SolutionState>(() => ({
  solution: null,
  projects: [],
  efProjects: [],
  dialog: null,
}));

const parentOf = (p: string) => p.slice(0, Math.max(0, p.lastIndexOf("/")));

export function projectsOf(items: readonly SolutionItem[], folders: string[] = []): SolutionProject[] {
  return items.flatMap((i): SolutionProject[] => {
    if (i.kind === "folder") return projectsOf(i.items, [...folders, i.name]);
    if (i.kind === "project" && !i.missing) return [{ name: i.name, path: i.path, dir: parentOf(i.path), folders }];
    return [];
  });
}

export function setSolution(solution: Solution | null) {
  useSolution.setState({
    solution,
    projects: solution ? projectsOf(solution.items).sort((a, b) => a.name.localeCompare(b.name)) : [],
  });
}

/** The project whose folder `dir` is. */
export const projectAt = (dir: string) => useSolution.getState().projects.find((p) => p.dir === dir) ?? null;

export const openDialog = (dialog: SolutionDialog) => useSolution.setState({ dialog });
export const closeDialog = () => useSolution.setState({ dialog: null });

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

async function edit(op: () => Promise<unknown>, done: string, failed: string): Promise<boolean> {
  try {
    await op();
    notifications().notify({ level: "info", title: done, group: "tools" });
    return true;
  } catch (e) {
    notifications().notify({
      level: "error",
      title: failed,
      body: message(e),
      group: "tools",
    });
    return false;
  }
}

export function removeFromSolution(projects: readonly SolutionProject[]): Promise<boolean> {
  const names = projects.map((p) => p.name).join(", ");
  return edit(
    () => call(commands.dotnetSolutionRemove(projects.map((p) => p.path))),
    tx`Removed ${names} from the solution`,
    tx`Cannot remove from the solution`,
  );
}

export function addToSolution(paths: readonly string[], folder: string | null): Promise<boolean> {
  const count = paths.length;
  return edit(
    () => call(commands.dotnetSolutionAdd([...paths], folder)),
    count === 1 ? tx`Added the project to the solution` : tx`Added ${count} projects to the solution`,
    tx`Cannot add to the solution`,
  );
}

export function editReferences(project: SolutionProject, add: string[], remove: string[]): Promise<boolean> {
  const name = project.name;
  return edit(
    () => call(commands.dotnetEditReferences(project.path, add, remove)),
    tx`Updated the references of ${name}`,
    tx`Cannot change the references of ${name}`,
  );
}

/** Converts the open `.sln` to `.slnx` (SLN-6); the Solution Explorer then shows the `.slnx`. */
export function migrateSolution(): Promise<boolean> {
  const solution = useSolution.getState().solution;
  if (!solution || solution.format !== "sln") return Promise.resolve(false);
  const name = solution.name;
  return edit(
    () => call(commands.dotnetMigrateSolution(solution.path)),
    tx`Converted ${name} to .slnx`,
    tx`Cannot convert the solution to .slnx`,
  );
}

/** Picks project files and adds them to the solution. */
export async function addExistingProject(solutionFolder: string | null = null): Promise<void> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: true,
    title: tx`Add Existing Project`,
    filters: [
      {
        name: tx`Project files`,
        extensions: ["csproj", "fsproj", "vbproj", "sqlproj", "esproj"],
      },
    ],
  });
  const paths = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter(Boolean);
  if (paths.length) await addToSolution(paths, solutionFolder);
}
