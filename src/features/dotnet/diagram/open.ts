/** Opens the project dependency diagram tab (kept apart so the diagram loads lazily). */
import { maybeEditorService } from "@/workbench/contributions";

export const DIAGRAM_KIND = "dotnet.projectDiagram";

export function openProjectDiagram() {
  void maybeEditorService()?.openInput({ kind: DIAGRAM_KIND, key: DIAGRAM_KIND, data: {} }, { focus: true });
}
