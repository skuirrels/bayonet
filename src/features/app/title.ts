/**
 * Window title, IDEA style: `<project> – <file>`; with Settings › Appearance ›
 * "Always show full path in window header" the file's full path.
 */
import { useEditorRegistry } from "@/editor/api";
import { t, useI18n } from "@/platform/i18n";
import { isTauri } from "@/platform/ipc";
import { basename, useProject } from "@/platform/project";

export function windowTitle(
  project: string | null | undefined,
  file: string | null | undefined,
  fullPath = false,
): string {
  const name = file ? (fullPath ? file : basename(file)) : null;
  if (!project) return name ?? t("app.title.default");
  return name ? `${project} – ${name}` : project;
}

export function installWindowTitle(): () => void {
  let last: string | null = null;
  let setNative: ((title: string) => void) | null = null;
  if (isTauri()) {
    void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
      const w = getCurrentWindow();
      setNative = (title) => void w.setTitle(title).catch(() => {});
      if (last !== null) setNative(last);
    });
  }
  const update = () => {
    const { project, settings } = useProject.getState();
    const fullPath = settings?.appearance?.fullPathInTitle === true;
    const title = windowTitle(project?.name, useEditorRegistry.getState().active?.ctx.path, fullPath);
    if (title === last) return;
    last = title;
    if (typeof document !== "undefined") document.title = title;
    setNative?.(title);
  };
  update();
  const unsubs = [useProject.subscribe(update), useEditorRegistry.subscribe(update), useI18n.subscribe(update)];
  return () => unsubs.forEach((u) => u());
}
