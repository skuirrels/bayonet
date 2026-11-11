import { executeAction, getAction, registerActions } from "@/platform/actions";
import { call, commands, isTauri } from "@/platform/ipc";
import { closeProject, confirmLeaveProject, openProject, useProject } from "@/platform/project";
import { tx } from "@/platform/i18n";
import { usePlatform } from "@/platform/platformInfo";
import { runSearchFieldCommand, showContextMenu, useContextMenuStore } from "@/ui";
import { showCustomizeToolbar } from "./MainToolbarDialog";
import { showQuickSwitch } from "./quickSwitch";
import { notify } from "./notificationStore";
import { NOTIFICATIONS_TOOL_WINDOW } from "./Notifications";
import { activateToolWindow } from "./toolwindows/store";
import { toggleTheme } from "./theme";
import { cancelTask, canCancel, useTasks } from "./tasks";
import { maybeEditorService, showStatusInfo } from "./contributions";

export async function openProjectDialog() {
  if (!isTauri()) {
    notify({
      level: "warning",
      title: "Opening folders requires the desktop app",
      body: "Run `pnpm tauri dev` to use the file system.",
    });
    return;
  }
  const defaultPath = useProject.getState().settings?.general?.defaultProjectDirectory?.trim() || undefined;
  const picked = await pickOpenTarget(defaultPath);
  if (!picked) return;
  const { path, directory } = picked;
  // A file other than a solution or project file opens in the editor (LightEdit without a project).
  if (!directory && !PROJECT_FILE.test(path) && getAction("file.openPaths")) {
    await executeAction("file.openPaths", [path]);
    return;
  }
  // The app feature applies "Open project in" (new / this window / ask); a solution or project
  // file opens its folder.
  if (getAction("file.openProjectPath")) {
    await executeAction("file.openProjectPath", path);
    return;
  }
  if (!(await confirmLeaveProject())) return;
  try {
    await openProject(path);
  } catch (e) {
    notify({ level: "error", title: "Could not open project", body: e instanceof Error ? e.message : String(e) });
  }
}

/** Solutions and project files File › Open… also takes. */
const PROJECT_EXTENSIONS = ["sln", "slnx", "slnf", "csproj", "fsproj", "vbproj"];
const PROJECT_FILE = new RegExp(`\\.(?:${PROJECT_EXTENSIONS.join("|")})$`, "i");

/** A picked path; `directory` comes from the picker, so it holds in a remote window too. */
interface Picked {
  path: string;
  directory: boolean;
}

/**
 * File › Open…: a folder, a solution, a project file or any other file. macOS has one panel
 * for all of them; elsewhere a file dialog picks either folders or files, so a menu asks which.
 */
async function pickOpenTarget(defaultPath: string | undefined): Promise<Picked | null> {
  if (usePlatform.getState().os === "macos") {
    try {
      return await call(commands.dialogPickOpenTarget(tx`Open a folder, solution or project`, defaultPath ?? null));
    } catch (e) {
      console.warn("open panel failed, falling back to the folder picker", e);
    }
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const pickFolder = async (): Promise<Picked | null> => {
    const picked = await open({ directory: true, multiple: false, title: tx`Open Folder`, defaultPath });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return path ? { path, directory: true } : null;
  };
  if (usePlatform.getState().os === "macos") return pickFolder();
  const pickFile = async (): Promise<Picked | null> => {
    const picked = await open({
      directory: false,
      multiple: false,
      title: tx`Open Solution or Project`,
      defaultPath,
      filters: [
        { name: tx`Solutions and Projects`, extensions: PROJECT_EXTENSIONS },
        { name: tx`All Files`, extensions: ["*"] },
      ],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return path ? { path, directory: false } : null;
  };
  return new Promise<Picked | null>((resolve) => {
    let chosen = false;
    const choose = (pick: () => Promise<Picked | null>) => () => {
      chosen = true;
      pick().then(resolve, () => resolve(null));
    };
    showContextMenu({ x: window.innerWidth / 2 - 120, y: window.innerHeight / 3 }, [
      { id: "folder", label: "Folder…", icon: "folder-open", run: choose(pickFolder) },
      { id: "file", label: "Solution or Project…", icon: "project-csharp", run: choose(pickFile) },
    ]);
    // The menu closes without a choice: nothing to open. It closes before it runs the chosen
    // item, so look once that has had its turn.
    const unsubscribe = useContextMenuStore.subscribe((s) => {
      if (s.menu) return;
      unsubscribe();
      queueMicrotask(() => {
        if (!chosen) resolve(null);
      });
    });
  });
}

/** Stop Background Processes (⇧⌘F2): the one running cancellable task, else a choice of them. */
export function stopBackgroundProcesses() {
  const running = useTasks.getState().tasks.filter(canCancel);
  if (!running.length) return showStatusInfo("No background processes to stop");
  if (running.length === 1) return cancelTask(running[0]!.id);
  showContextMenu({ x: window.innerWidth / 2 - 160, y: 96 }, [
    ...running.map((t) => ({ id: String(t.id), label: t.title, icon: "stop", run: () => cancelTask(t.id) })),
    { kind: "separator" as const },
    { id: "all", label: "Stop All", run: () => running.forEach((t) => cancelTask(t.id)) },
  ]);
}

/** Generic actions owned by the shell. */
export function registerShellActions(): () => void {
  return registerActions([
    {
      id: "file.openProject",
      title: "Open…",
      category: "File",
      icon: "folder-open",
      keywords: ["folder", "project", "solution", "sln", "slnx", "csproj"],
      run: openProjectDialog,
    },
    {
      id: "file.closeProject",
      title: "Close Project",
      category: "File",
      when: "projectOpen",
      run: async () => {
        if (!useProject.getState().project || !(await confirmLeaveProject())) return;
        try {
          await closeProject();
        } catch (e) {
          maybeEditorService()?.cancelClose?.();
          throw e;
        }
      },
    },
    {
      id: "workbench.stopBackgroundProcesses",
      title: "Stop Background Processes…",
      category: "Window",
      keywords: ["cancel task", "background tasks", "progress"],
      run: stopBackgroundProcesses,
    },
    {
      id: "view.toggleTheme",
      title: "Toggle Light/Dark Theme",
      category: "View",
      icon: "sun",
      keywords: ["appearance", "theme"],
      run: toggleTheme,
    },
    {
      id: "view.quickSwitchScheme",
      title: "Quick Switch Scheme…",
      category: "View",
      keywords: ["theme", "color scheme", "keymap", "view mode"],
      run: () => showQuickSwitch({ x: window.innerWidth / 2 - 120, y: 96 }),
    },
    {
      id: "toolbar.customize",
      title: "Customize Main Toolbar…",
      category: "View",
      keywords: ["toolbar", "widgets", "title bar"],
      run: showCustomizeToolbar,
    },
    {
      id: "view.notifications",
      title: "Notifications",
      category: "View",
      icon: "bell",
      run: () => activateToolWindow(NOTIFICATIONS_TOOL_WINDOW),
    },
    // Keys of search fields (find bar, Find in Files) that are editor shortcuts elsewhere.
    {
      id: "find.field.newLine",
      title: "New Line in Search Field",
      category: "Find",
      hidden: true,
      when: "searchFieldNewLine",
      run: () => void runSearchFieldCommand("newLine"),
    },
    {
      id: "find.field.recentSearches",
      title: "Recent Searches",
      category: "Find",
      hidden: true,
      when: "searchFieldHistory",
      run: () => void runSearchFieldCommand("history"),
    },
  ]);
}
