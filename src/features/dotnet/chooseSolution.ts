/**
 * Project › Choose Solution…: the solution of a folder that holds several, its own or in
 * its subfolders (a folder of repositories). Build, tests, NuGet and the Solution pane use it.
 */
import { commands } from "@/bindings";
import { openPopup, topCentreAnchor, type ListPopupItem } from "@/editor/lsp/popupStore";
import { executeAction } from "@/platform/actions";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import { basename, relativeDir, useProject } from "@/platform/project";
import { notifications } from "@/workbench/contributions";

/** One row per solution, the current one marked, its folder as the detail. */
export function solutionItems(
  root: string,
  choices: readonly string[],
  current: string | null,
  pick: (path: string) => void,
): ListPopupItem[] {
  return choices.map((path) => ({
    id: path,
    label: basename(path),
    detail: relativeDir(root, path) || undefined,
    icon: path === current ? "check" : "module",
    emphasis: path === current,
    run: () => pick(path),
  }));
}

function failed(e: unknown) {
  notifications().notify({
    level: "error",
    title: tx("Cannot choose the solution"),
    body: e instanceof Error ? e.message : String(e),
    group: "dotnet",
  });
}

/** Opens the chooser; `chosen` reloads the solution once one is picked. */
export async function chooseSolution(chosen: () => void): Promise<void> {
  const root = useProject.getState().project?.root;
  if (!root) return;
  let choices;
  try {
    choices = await call(commands.dotnetSolutionChoices());
  } catch (e) {
    return failed(e);
  }
  if (!choices.solutions.length) {
    notifications().notify({
      level: "info",
      title: tx("No solutions in this folder or its subfolders"),
      group: "dotnet",
    });
    return;
  }
  const pick = async (path: string) => {
    try {
      await call(commands.dotnetUseSolution(path));
    } catch (e) {
      return failed(e);
    }
    chosen();
    // Run configurations come from the solution's projects.
    void executeAction("run.reloadConfigs");
  };
  openPopup({
    kind: "list",
    anchor: topCentreAnchor(),
    title: tx("Choose Solution"),
    items: solutionItems(root, choices.solutions, choices.current, (path) => void pick(path)),
  });
}
