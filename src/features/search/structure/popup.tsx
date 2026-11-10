import { create } from "zustand";
import { lazyComponent } from "@/ui";

/** Whether the ⌘F12 File Structure popup is open. */
export const useFileStructurePopup = create<{ open: boolean }>(() => ({ open: false }));

const FileStructureDialog = lazyComponent(() => import("./StructureView").then((m) => m.FileStructureOverlay));

/** The popup's overlay: its code loads when it first opens (or while idle), not at start-up. */
export function FileStructurePopup() {
  const open = useFileStructurePopup((s) => s.open);
  return open ? <FileStructureDialog /> : null;
}
