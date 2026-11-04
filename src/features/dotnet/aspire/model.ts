/**
 * The Aspire window's model (BLD-5): resources of the running AppHost as
 * `aspire describe` reports them, how each state reads, and the console
 * buffer per resource.
 */
import type { AspireLogLine, AspireResource } from "@/bindings";
import type { RunConfig } from "@/platform/ipc";
import { parseDotnetRun } from "@/platform/dotnetRun";

/** Lines kept per resource console. */
export const MAX_LOG_LINES = 5000;

export type ResourceTone = "running" | "starting" | "warning" | "failed" | "stopped";

/** How a resource's state reads: its dot colour and whether it is up. */
export function resourceTone(r: Pick<AspireResource, "state" | "health" | "exitCode">): ResourceTone {
  const state = (r.state ?? "").toLowerCase();
  if (state === "running") {
    const health = (r.health ?? "").toLowerCase();
    return health === "unhealthy" || health === "degraded" ? "warning" : "running";
  }
  if (["starting", "waiting", "notstarted", "building", "stopping", "activating", "recovering"].includes(state))
    return "starting";
  if (state === "failedtostart" || state === "runtimeunhealthy") return "failed";
  if ((state === "exited" || state === "finished") && r.exitCode != null && r.exitCode !== 0) return "failed";
  return "stopped";
}

/** `FailedToStart` → `Failed to start`; a missing state reads as unknown. */
export function stateLabel(state: string | null): string {
  if (!state) return "Unknown";
  const words = state.replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The list with `r` added or replaced (by name), sorted by display name then name. */
export function upsertResource(list: readonly AspireResource[], r: AspireResource): AspireResource[] {
  const next = list.filter((x) => x.name !== r.name);
  next.push(r);
  return next.sort((a, b) => a.displayName.localeCompare(b.displayName) || a.name.localeCompare(b.name));
}

/** Replicas share a display name: show the unique name for them. */
export function resourceLabel(r: AspireResource, all: readonly AspireResource[]): string {
  return all.filter((x) => x.displayName === r.displayName).length > 1 ? r.name : r.displayName;
}

/** The console buffer with `batch` appended, keeping the last `max` lines. */
export function appendLogs(lines: readonly AspireLogLine[], batch: readonly AspireLogLine[], max = MAX_LOG_LINES) {
  const next = lines.concat(batch);
  return next.length > max ? next.slice(next.length - max) : next;
}

/** Commands shown as buttons: lifecycle ones that make sense in this state. */
export function availableCommands(r: AspireResource): AspireResource["commands"] {
  const up = resourceTone(r) !== "stopped" && resourceTone(r) !== "failed";
  return r.commands.filter((c) => (c.name === "start" ? !up : c.name === "stop" ? up : true));
}

const norm = (p: string) => p.replace(/\\/g, "/").replace(/^\.\//, "");

/** The detected `dotnet run` configuration of `apphost` (its first launch profile). */
export function appHostConfig(apphost: string, root: string, configs: readonly RunConfig[]): RunConfig | undefined {
  const rel = norm(apphost.startsWith(`${root}/`) ? apphost.slice(root.length + 1) : apphost);
  return configs.find((c) => {
    const run = parseDotnetRun(c.command, c.args);
    return !!run?.project && (norm(run.project) === rel || norm(run.project) === norm(apphost));
  });
}
