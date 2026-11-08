/**
 * A project's Dependencies in the Solution view (SLN-1), as Rider shows them:
 * Frameworks (shared frameworks), Packages (NuGet packages, each with the
 * packages it brings in) and Projects (project references; opening one selects
 * that project). A multi-targeted project groups them by target framework.
 * The backend reads restore's `project.assets.json`, or the project file before
 * the first restore.
 */
import { commands, type FrameworkDependencies, type PackageDependency, type ProjectDependencies } from "@/bindings";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import type { ProjectViewRootEntry } from "@/workbench/contributions";
import { parentOf } from "@/features/project-tree/paths";

/** Dependencies per project file. */
export type DependencyMap = Readonly<Record<string, ProjectDependencies>>;

/** Package nodes per framework, transitive ones included; beyond it, deeper levels are left out. */
const MAX_PACKAGE_NODES = 2000;

/** Restore output: a change can change the dependencies. */
export const touchesDependencies = (paths: readonly string[]) => paths.some((p) => /\/project\.assets\.json$/i.test(p));

export async function loadDependencies(projects: readonly string[]): Promise<DependencyMap> {
  if (!projects.length) return {};
  const all = await call(commands.dotnetDependencies([...projects]));
  const out: Record<string, ProjectDependencies> = {};
  projects.forEach((p, i) => {
    const d = all[i];
    if (d) out[p] = d;
  });
  return out;
}

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const projectName = (path: string) => fileName(path).replace(/\.[^.]+$/, "");

function packageNodes(fw: FrameworkDependencies): ProjectViewRootEntry[] {
  const byId = new Map(fw.packages.map((p) => [p.id.toLowerCase(), p]));
  let budget = MAX_PACKAGE_NODES;
  const node = (p: PackageDependency, ancestors: ReadonlySet<string>): ProjectViewRootEntry => {
    budget--;
    const lower = p.id.toLowerCase();
    const inner = new Set(ancestors).add(lower);
    const children = p.dependencies
      .map((d) => byId.get(d.toLowerCase()))
      .filter((d): d is PackageDependency => !!d && !inner.has(d.id.toLowerCase()));
    return {
      id: p.id,
      label: p.id,
      icon: "package",
      detail: p.version ?? undefined,
      children: budget > 0 ? children.map((c) => node(c, inner)) : [],
    };
  };
  return fw.packages.filter((p) => p.topLevel).map((p) => node(p, new Set()));
}

function frameworkNodes(fw: FrameworkDependencies, restored: boolean): ProjectViewRootEntry[] {
  const groups: ProjectViewRootEntry[] = [];
  if (fw.frameworkReferences.length)
    groups.push({
      id: "frameworks",
      label: tx("Frameworks"),
      icon: "library",
      children: fw.frameworkReferences.map((f) => ({ id: f, label: f, icon: "library", children: [] })),
    });
  if (fw.packages.length)
    groups.push({
      id: "packages",
      label: tx("Packages"),
      icon: "toolwindow-nuget",
      detail: restored ? undefined : tx("not restored"),
      children: packageNodes(fw),
    });
  if (fw.projects.length)
    groups.push({
      id: "projects",
      label: tx("Projects"),
      icon: "module",
      children: fw.projects.map((p) => ({
        id: p,
        label: projectName(p),
        icon: /\.fsproj$/i.test(p) ? "project-fsharp" : /\.csproj$/i.test(p) ? "project-csharp" : "module",
        reveal: parentOf(p),
        children: [],
      })),
    });
  return groups;
}

/** Frameworks, Packages and Projects of `deps`; one group per target framework when there are several. */
export function dependencyEntries(deps: ProjectDependencies | undefined): ProjectViewRootEntry[] {
  if (!deps) return [];
  const [only, ...more] = deps.frameworks;
  if (!only) return [];
  if (!more.length) return frameworkNodes(only, deps.restored);
  return deps.frameworks.map((fw) => ({
    id: `tfm:${fw.framework}`,
    label: fw.framework,
    icon: "library",
    children: frameworkNodes(fw, deps.restored),
  }));
}
