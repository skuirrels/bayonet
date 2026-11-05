/**
 * Manage User Secrets (PLT-4): opens the project's `secrets.json` (outside the
 * source tree, where the configuration provider reads it) in the editor. A project
 * without a `UserSecretsId` gets one from `dotnet user-secrets init` first.
 */
import { commands } from "@/bindings";
import { activeEditor } from "@/editor/api";
import { tx } from "@/platform/i18n";
import { call, isTauri } from "@/platform/ipc";
import { maybeEditorService, notifications } from "@/workbench/contributions";
import { projectAt, useSolution, type SolutionProject } from "./projects/store";

/** The project whose folder holds `path` (the innermost one). */
export function projectContaining(projects: readonly SolutionProject[], path: string): SolutionProject | null {
  let best: SolutionProject | null = null;
  for (const p of projects) if (path.startsWith(`${p.dir}/`) && (!best || p.dir.length > best.dir.length)) best = p;
  return best;
}

/** The project of `dir`, else of the active editor's file, else the only one. */
export function targetProject(dir: string | null): SolutionProject | null {
  if (dir) return projectAt(dir);
  const { projects } = useSolution.getState();
  const path = activeEditor()?.ctx.path;
  return (path ? projectContaining(projects, path) : null) ?? (projects.length === 1 ? projects[0]! : null);
}

export async function manageUserSecrets(dir: string | null): Promise<void> {
  if (!isTauri()) return;
  const project = targetProject(dir);
  if (!project) {
    notifications().notify({
      group: "dotnet",
      level: "info",
      title: tx`Select a project in the Solution Explorer to manage its user secrets`,
    });
    return;
  }
  try {
    const file = await call(commands.dotnetUserSecrets(project.path));
    await maybeEditorService()?.openFile(file, { focus: true });
  } catch (e) {
    const name = project.name;
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Cannot open the user secrets of ${name}`,
      body: e instanceof Error ? e.message : String(e),
    });
  }
}
