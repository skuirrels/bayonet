/**
 * NuGet restore outside builds (NUG-1). When a solution opens, and when a
 * project file or a `Directory.*.props` / `nuget.config` changes, the projects
 * whose restore is missing or out of date are restored in the background; so
 * are the projects Roslyn reports with unresolved packages. Restores run one at
 * a time: targets asked for meanwhile are gathered into the next one.
 *
 * Progress is a cancellable background task. Errors and warnings show in the
 * Problems view on the project file they name (`nuget` diagnostics, replaced
 * by the next restore); a failed restore also notifies.
 */
import { commands, type RequestId, type RestoreMessage, type RestoreResult } from "@/bindings";
import { getContext, useContextKeys } from "@/platform/context";
import { tx } from "@/platform/i18n";
import { call, isCancelled, nextRequestId } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { maybeEditorService, notifications, toolWindowService } from "@/workbench/contributions";
import { removeTask, upsertTask } from "@/workbench/tasks";

export const RESTORE_TASK_ID = "dotnet.restore";

/** Files a restore changes the inputs of; their changes ask for a restore check. */
const RESTORE_INPUT =
  /(?:\.(?:cs|fs|vb)proj|(?:^|\/)(?:Directory\.(?:Packages|Build)\.(?:props|targets)|nuget\.config))$/i;

export const touchesRestoreInputs = (paths: readonly string[]) => paths.some((p) => RESTORE_INPUT.test(p));

/**
 * Settings › Languages › .NET › Restore NuGet packages automatically (on unless
 * turned off), in a trusted project: Restricted Mode runs no tools.
 */
export const autoRestore = () => {
  const { project, settings } = useProject.getState();
  return !!project?.trusted && settings?.languages?.dotnet?.restoreOnOpen !== false;
};

export interface RestoreBackend {
  needed(projects: string[]): Promise<string[]>;
  restore(targets: string[], clear: string[], request: RequestId): Promise<RestoreResult>;
  cancel(request: RequestId): void;
}

const tauriBackend: RestoreBackend = {
  needed: (projects) => call(commands.dotnetRestoreNeeded(projects)),
  restore: (targets, clear, request) => call(commands.dotnetRestore(targets, clear, request)),
  cancel: (request) => void commands.cancelRequest(request),
};

let backend = tauriBackend;

/** Tests swap the backend; `null` restores the real one. */
export function setRestoreBackend(b: RestoreBackend | null) {
  backend = b ?? tauriBackend;
}

let pending = new Set<string>();
let running: Promise<void> | null = null;
let current: RequestId | null = null;
/** What the running restore restores. */
let restoring = new Set<string>();
/** Files that have `nuget` diagnostics, cleared by the next restore that covers them. */
let flagged = new Set<string>();
/** Targets restored since the project opened. */
let restored = new Set<string>();
/** Checks put off while a build runs (it restores too): projects and their solution. */
let deferred: { projects: Set<string>; solution: string | null; requested: boolean } | null = null;
let epoch = 0;

/** The Build feature's context key while a build runs. */
const BUILD_RUNNING = "build.running";

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const isSolution = (path: string) => /\.slnx?$|\.slnf$/i.test(path);

/** Restores `targets` (project or solution files) after any restore already running. */
export function restore(targets: readonly string[]): Promise<void> {
  for (const t of targets) pending.add(t);
  if (!running) {
    const drained: Promise<void> = drain(epoch).finally(() => {
      if (running === drained) running = null;
    });
    running = drained;
  }
  return running;
}

/**
 * Restores those of `projects` that need it and are not being restored: the
 * solution when more than one does (one `dotnet restore` instead of one per
 * project), else that project. With `requested` (Roslyn found unresolved
 * packages), also those not restored since the project opened, though their
 * restore looks current. Put off while a build runs. Off with Settings ›
 * Languages › .NET.
 */
export async function restoreIfNeeded(
  projects: readonly string[],
  solution: string | null,
  requested = false,
): Promise<void> {
  if (!projects.length || !autoRestore()) return;
  if (getContext()[BUILD_RUNNING]) {
    deferred = {
      projects: new Set([...(deferred?.projects ?? []), ...projects]),
      solution: solution ?? deferred?.solution ?? null,
      requested: requested || !!deferred?.requested,
    };
    return;
  }
  const id = epoch;
  const stale = await backend.needed([...new Set(projects)]).catch((e: unknown) => {
    console.warn("restore: cannot check which projects need a restore", e);
    return [] as string[];
  });
  if (id !== epoch) return;
  const covered = (p: string, by: ReadonlySet<string>) => by.has(p) || (!!solution && by.has(solution));
  const todo = [...new Set(projects)].filter(
    (p) =>
      (stale.includes(p) || (requested && !covered(p, restored))) && !covered(p, restoring) && !covered(p, pending),
  );
  if (!todo.length) return;
  await restore(todo.length > 1 && solution ? [solution] : todo);
}

/**
 * Restores `projects` again even when restored since the project opened: what restore
 * resolves changed without a file changing (Settings › .NET › Environment variables).
 */
export function restoreAgain(projects: readonly string[], solution: string | null): Promise<void> {
  restored = new Set();
  return restoreIfNeeded(projects, solution, true);
}

/** Runs the checks put off by a build once it ends; returns the unsubscribe. */
export function watchBuilds(): () => void {
  return useContextKeys.subscribe((c, prev) => {
    if (!prev[BUILD_RUNNING] || c[BUILD_RUNNING] || !deferred) return;
    const { projects, solution, requested } = deferred;
    deferred = null;
    void restoreIfNeeded([...projects], solution, requested);
  });
}

async function drain(id: number) {
  while (id === epoch && pending.size) {
    const targets = [...pending];
    pending = new Set();
    await run(targets);
  }
}

async function run(targets: string[]) {
  const id = epoch;
  const request = nextRequestId();
  current = request;
  restoring = new Set(targets);
  const title = tx("Restoring NuGet packages");
  upsertTask({
    id: RESTORE_TASK_ID,
    title,
    message: targets.map(fileName).join(", "),
    fraction: null,
    done: false,
    cancel: () => backend.cancel(request),
  });
  // A solution's restore covers every flagged file; a project's, that project's.
  const clear = targets.some(isSolution) ? [...flagged] : targets.filter((t) => flagged.has(t));
  try {
    const result = await backend.restore(targets, clear, request);
    if (id !== epoch) return;
    for (const f of clear) flagged.delete(f);
    for (const m of result.messages) if (m.project) flagged.add(m.project);
    for (const t of targets) restored.add(t);
    if (!result.success) notifyFailure(result.messages);
  } catch (e) {
    if (id !== epoch || isCancelled(e)) return;
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx("NuGet restore failed"),
      body: e instanceof Error ? e.message : String(e),
    });
  } finally {
    if (current === request) {
      current = null;
      restoring = new Set();
    }
    if (id === epoch) upsertTask({ id: RESTORE_TASK_ID, title, message: null, fraction: 1, done: true });
  }
}

function notifyFailure(messages: readonly RestoreMessage[]) {
  const errors = messages.filter((m) => m.severity === "error");
  const first = errors[0];
  const project = first?.project ?? null;
  notifications().notify({
    group: "dotnet",
    level: "error",
    key: "nuget-restore-failed",
    title:
      errors.length === 1 ? tx("NuGet restore failed: 1 error") : tx`NuGet restore failed: ${errors.length} errors`,
    body: first ? `${first.code ? `${first.code}: ` : ""}${first.text}` : undefined,
    actions: [
      { label: tx("Show Problems"), run: () => toolWindowService().show("problems", true) },
      ...(project
        ? [
            {
              label: tx`Open ${fileName(project)}`,
              run: () => void maybeEditorService()?.openFile(project, { focus: true }),
            },
          ]
        : []),
    ],
  });
}

/** Forgets queued restores and stops the running one (project closed or switched). */
export function resetRestore() {
  epoch++;
  pending = new Set();
  running = null;
  flagged = new Set();
  restored = new Set();
  deferred = null;
  if (current !== null) backend.cancel(current);
  current = null;
  restoring = new Set();
  removeTask(RESTORE_TASK_ID);
}
