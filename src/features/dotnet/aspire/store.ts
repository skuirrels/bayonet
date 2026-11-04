/**
 * The Aspire window (BLD-5): the solution's AppHosts, whether the chosen one
 * runs, its resources (streamed while the window is open) and the console of
 * the selected resource. Starting the AppHost goes through its detected
 * `dotnet run` configuration, so it shows in the Run window as usual.
 */
import { create } from "zustand";
import { Channel } from "@tauri-apps/api/core";
import {
  commands,
  type AspireAppHost,
  type AspireCli,
  type AspireEvent,
  type AspireLogLine,
  type AspireResource,
  type RequestId,
} from "@/bindings";
import { executeAction } from "@/platform/actions";
import { tx } from "@/platform/i18n";
import { call, isCancelled, isTauri, nextRequestId } from "@/platform/ipc";
import { openExternalUrl } from "@/platform/openUrl";
import { useProject } from "@/platform/project";
import { notifications } from "@/workbench/contributions";
import { loadConfigs, useRun } from "@/features/run/store";
import { appHostConfig, appendLogs, upsertResource } from "./model";
import { attachDotnetDebugger } from "../attach";

interface AspireState {
  /** AppHost projects of the solution. */
  apphosts: string[];
  selected: string | null;
  cli: AspireCli | null;
  /** The selected AppHost while it runs. */
  running: AspireAppHost | null;
  /** Whether `running` is known yet (the first poll answered). */
  known: boolean;
  watching: RequestId | null;
  resources: AspireResource[];
  resource: string | null;
  logs: AspireLogLine[];
  logsRequest: RequestId | null;
  /** `<resource>:<command>` being run. */
  busy: ReadonlySet<string>;
  starting: boolean;
  error: string | null;
}

const initial = (): AspireState => ({
  apphosts: [],
  selected: null,
  cli: null,
  running: null,
  known: false,
  watching: null,
  resources: [],
  resource: null,
  logs: [],
  logsRequest: null,
  busy: new Set(),
  starting: false,
  error: null,
});

export const useAspire = create<AspireState>(initial);

/** How long the window shows "Starting" before giving up on the AppHost appearing. */
const START_TIMEOUT = 120_000;

const get = () => useAspire.getState();
const set = (patch: Partial<AspireState> | ((s: AspireState) => Partial<AspireState>)) => useAspire.setState(patch);
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The AppHosts among the solution's `projects`; returns them. */
export async function loadAppHosts(projects: readonly string[]): Promise<string[]> {
  if (!isTauri()) return [];
  const apphosts = projects.length ? await call(commands.aspireApphosts([...projects])) : [];
  const selected = get().selected;
  set({ apphosts, selected: selected && apphosts.includes(selected) ? selected : (apphosts[0] ?? null) });
  return apphosts;
}

export async function loadCli(): Promise<AspireCli> {
  const cli = await call(commands.aspireCli());
  set({ cli });
  return cli;
}

/** Watches the selected AppHost (while the window is open). */
export async function startWatch() {
  const apphost = get().selected;
  if (!apphost || !isTauri() || get().watching != null) return;
  const cli = get().cli ?? (await loadCli());
  if (!cli.path) return;
  const request = nextRequestId();
  const channel = new Channel<AspireEvent>();
  channel.onmessage = (e) => {
    if (get().watching !== request) return;
    if (e.kind === "appHost") {
      const wasRunning = !!get().running;
      set({ running: e.running, known: true, starting: false });
      if (!e.running) {
        stopLogs();
        set({ resources: [] });
      } else if (!wasRunning && get().resource) void startLogs();
    } else if (e.kind === "resource") {
      set((s) => ({ resources: upsertResource(s.resources, e.resource) }));
    } else {
      set({ error: e.message });
    }
  };
  set({ watching: request, error: null, known: false });
  try {
    await call(commands.aspireWatch(request, apphost, channel));
  } catch (e) {
    if (!isCancelled(e) && get().watching === request) set({ error: message(e) });
  } finally {
    if (get().watching === request) set({ watching: null });
  }
}

export function stopWatch() {
  stopLogs();
  const request = get().watching;
  if (request == null) return;
  set({ watching: null });
  void commands.cancelRequest(request);
}

export function selectAppHost(apphost: string) {
  if (apphost === get().selected) return;
  stopWatch();
  set({ selected: apphost, running: null, known: false, resources: [], resource: null, logs: [] });
  void startWatch();
}

/** Shows the console of `resource`. */
export function selectResource(resource: string | null) {
  if (resource === get().resource) return;
  stopLogs();
  set({ resource, logs: [] });
  if (resource && get().running) void startLogs();
}

async function startLogs() {
  const { selected: apphost, resource } = get();
  if (!apphost || !resource || get().logsRequest != null) return;
  const request = nextRequestId();
  const channel = new Channel<AspireLogLine[]>();
  channel.onmessage = (batch) => {
    if (get().logsRequest === request) set((s) => ({ logs: appendLogs(s.logs, batch) }));
  };
  set({ logsRequest: request, logs: [] });
  try {
    await call(commands.aspireLogs(request, apphost, resource, channel));
  } catch (e) {
    if (!isCancelled(e) && get().logsRequest === request) set({ error: message(e) });
  } finally {
    if (get().logsRequest === request) set({ logsRequest: null });
  }
}

function stopLogs() {
  const request = get().logsRequest;
  if (request == null) return;
  set({ logsRequest: null });
  void commands.cancelRequest(request);
}

/** Runs `start`, `stop`, `restart`, `rebuild` or a custom command of `resource`. */
export async function runCommand(resource: AspireResource, name: string) {
  const apphost = get().selected;
  const key = `${resource.name}:${name}`;
  if (!apphost || get().busy.has(key)) return;
  set((s) => ({ busy: new Set(s.busy).add(key) }));
  try {
    await call(commands.aspireCommand(apphost, resource.name, name));
  } catch (e) {
    const label = resource.displayName;
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Aspire: ${name} failed for ${label}`,
      body: message(e),
    });
  } finally {
    set((s) => {
      const busy = new Set(s.busy);
      busy.delete(key);
      return { busy };
    });
  }
}

/** Starts the AppHost through its run configuration (so it shows in the Run window). */
export async function runAppHost() {
  const apphost = get().selected;
  const root = useProject.getState().project?.root;
  if (!apphost || !root) return;
  if (!useRun.getState().configs.length) await loadConfigs();
  const config = appHostConfig(apphost, root, useRun.getState().configs);
  if (!config) {
    set({ error: tx`No run configuration for this AppHost` });
    return;
  }
  set({ starting: true, error: null });
  const ran = await executeAction("run.runConfig", config.id);
  if (!ran) set({ starting: false });
  // A start that never shows up in `aspire ps` (a build error, say) stops spinning eventually.
  else setTimeout(() => set((s) => (s.starting && !s.running ? { starting: false } : {})), START_TIMEOUT);
}

/** Attaches the debugger to a running project resource (DBG-5). */
export async function debugResource(resource: AspireResource) {
  if (resource.pid == null) return;
  const root = useProject.getState().project?.root ?? "";
  const cwd = resource.projectPath ? resource.projectPath.replace(/[\\/][^\\/]*$/, "") : root;
  try {
    await attachDotnetDebugger(resource.pid, resource.displayName, cwd);
  } catch (e) {
    set({ error: message(e) });
  }
}

export async function stopAppHost() {
  const apphost = get().selected;
  if (!apphost) return;
  try {
    await call(commands.aspireStop(apphost));
  } catch (e) {
    set({ error: message(e) });
  }
}

export function openDashboard(url?: string | null) {
  const target = url ?? get().running?.dashboardUrl;
  if (target) void openExternalUrl(target);
}

/** Installs the CLI in the terminal (`dotnet tool install --global Aspire.Cli`). */
export function installCli() {
  const install = get().cli?.install;
  if (!install) return;
  void executeAction("terminal.runCommand", {
    program: install.command,
    args: install.args,
    name: tx`Install Aspire CLI`,
  });
}

export function resetAspire() {
  stopWatch();
  useAspire.setState(initial());
}
