/**
 * .NET: the Solution pane of the Project view, from the `.slnx` / `.sln` /
 * `.slnf` at the project root (read by the backend, no MSBuild evaluation).
 */
import { commands } from "@/bindings";
import { registerActions } from "@/platform/actions";
import { call, isTauri, onIdeEvent } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { registerProjectViewRoot } from "@/workbench/contributions";
import { GROUP_PREFIX } from "@/features/project-tree/model";
import { setExpanded, suggestViewMode, useProjectTree } from "@/features/project-tree/store";
import { SOLUTION_ROOT_ID, solutionRoot, touchesSolution } from "./solution";

const REFRESH_DELAY = 300;

export function activate(): () => void {
  let disposeRoot = () => {};
  let seq = 0;
  let shown = "";
  const clear = () => {
    disposeRoot();
    disposeRoot = () => {};
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
      clear();
      shown = key;
      disposeRoot = registerProjectViewRoot(solutionRoot(solution));
      if (first) {
        suggestViewMode("solution");
        setExpanded(new Set([...useProjectTree.getState().expanded, `${GROUP_PREFIX}${SOLUTION_ROOT_ID}`]));
      }
    } catch (e) {
      if (id === seq) console.warn("solution: cannot read the solution", e);
    }
  };
  void refresh();
  const disposeActions = registerActions([
    { id: "dotnet.reloadSolution", title: "Reload Solution", category: "Project", when: "projectOpen", run: refresh },
  ]);
  const unsub = useProject.subscribe((s, prev) => {
    if (s.project?.root !== prev.project?.root) void refresh();
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const unsubFs = onIdeEvent("fsChanged", (e) => {
    if (!touchesSolution([...e.created, ...e.modified, ...e.removed])) return;
    clearTimeout(timer);
    timer = setTimeout(() => void refresh(), REFRESH_DELAY);
  });
  return () => {
    seq++;
    clearTimeout(timer);
    unsub();
    unsubFs();
    disposeActions();
    clear();
  };
}
