import { editorsFor } from "@/editor/api";
import { call, commands } from "@/platform/ipc";
import type { RequestId } from "@/platform/ipc";

export { call, commands, isTauri, nextRequestId, onIdeEvent } from "@/platform/ipc";

export function cancelRequestSafe(id: RequestId) {
  try {
    void commands.cancelRequest(id).catch(() => {});
  } catch {
    // not running under Tauri
  }
}

/** File text for the preview pane: the open buffer, else the file on disk. */
export function previewText(path: string): Promise<string> {
  const view = editorsFor(path)[0]?.view;
  return view ? Promise.resolve(view.state.doc.toString()) : call(commands.fsReadText(path));
}
