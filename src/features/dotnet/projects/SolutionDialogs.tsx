/** Hosts the solution dialogs (New Project, Project References) as an overlay. */
import { NewProjectDialog } from "./NewProjectDialog";
import { ReferencesDialog } from "./ReferencesDialog";
import { useSolution } from "./store";

export function SolutionDialogs() {
  const dialog = useSolution((s) => s.dialog);
  if (!dialog) return null;
  return dialog.kind === "newProject" ? (
    <NewProjectDialog directory={dialog.directory} solutionFolder={dialog.solutionFolder ?? null} />
  ) : (
    <ReferencesDialog project={dialog.project} />
  );
}
