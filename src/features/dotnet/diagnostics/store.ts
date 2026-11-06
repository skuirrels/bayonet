/**
 * .NET Diagnostics tool window state (DIAG): the process being diagnosed, its
 * live counters (from the dotnet-monitor sidecar), what was collected from it and
 * the trigger rules that collect on their own when a counter crosses a threshold.
 */
import { create } from "zustand";
import { Channel } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import type {
  AttachableProcess,
  CounterRecording,
  DiagArtifact,
  DiagTool,
  DiagToolStatus,
  DumpAnalysis,
  DumpType,
  HeapSnapshot,
  MetricSample,
  RequestId,
  StackSnapshot,
  TraceProfile,
} from "@/bindings";
import { call, isCancelled, isTauri, nextRequestId } from "@/platform/ipc";
import { executeAction } from "@/platform/actions";
import { useProject } from "@/platform/project";
import { tx } from "@/platform/i18n";
import { maybeEditorService, notifications } from "@/workbench/contributions";
import {
  addSamples,
  DEFAULT_RULES,
  processGroup,
  ruleFires,
  type Series,
  type TriggerAction,
  type TriggerRule,
} from "./model";
import { setTraceHeat } from "./heat";
import { heatByFile, lineHeatByFile } from "./trace";

export type DiagTab = "live" | "threads" | "memory" | "dumps" | "trace" | "triggers" | "files" | "tools";
/** How the CPU Trace tab shows the trace. */
export type TraceView = "flame" | "tree" | "methods";
export type Job = "stacks" | "gcdump" | "dump" | "analyze" | "trace" | "counters" | "symbols";

export interface Firing {
  rule: string;
  time: number;
  action: TriggerAction;
  /** What it collected, or why it could not. */
  result: string;
}

export interface DiagnosticsState {
  tab: DiagTab;
  tools: DiagToolStatus[] | null;
  processes: AttachableProcess[];
  loadingProcesses: boolean;
  pid: number | null;
  /** The live counter stream's request while it runs. */
  live: RequestId | null;
  livePid: number | null;
  series: Record<string, Series>;
  /** The counter the Live tab charts large. */
  selectedSeries: string | null;
  stacks: StackSnapshot | null;
  heap: HeapSnapshot | null;
  /** The snapshot `heap` is compared with. */
  baseline: HeapSnapshot | null;
  dump: DumpAnalysis | null;
  trace: TraceProfile | null;
  traceView: TraceView;
  /** The method (frame name) the CPU Trace tab shows callers and callees of. */
  traceMethod: string | null;
  counters: CounterRecording | null;
  busy: Partial<Record<Job, RequestId>>;
  artifacts: DiagArtifact[];
  traceSeconds: number;
  dumpType: DumpType;
  rules: TriggerRule[];
  /** Last firing time by rule id. */
  fired: Record<string, number>;
  firings: Firing[];
  error: string | null;
}

const RULES_KEY = "openidea.dotnet.diagnostics.rules";

function loadRules(): TriggerRule[] {
  try {
    const raw = globalThis.localStorage?.getItem(RULES_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    if (Array.isArray(parsed)) return parsed as TriggerRule[];
  } catch {
    // Unavailable storage: the defaults.
  }
  return DEFAULT_RULES;
}

function saveRules(rules: TriggerRule[]) {
  try {
    globalThis.localStorage?.setItem(RULES_KEY, JSON.stringify(rules));
  } catch {
    // Not persisted.
  }
}

const initial = (): DiagnosticsState => ({
  tab: "live",
  tools: null,
  processes: [],
  loadingProcesses: false,
  pid: null,
  live: null,
  livePid: null,
  series: {},
  selectedSeries: null,
  stacks: null,
  heap: null,
  baseline: null,
  dump: null,
  trace: null,
  traceView: "flame",
  traceMethod: null,
  counters: null,
  busy: {},
  artifacts: [],
  traceSeconds: 10,
  dumpType: "heap",
  rules: loadRules(),
  fired: {},
  firings: [],
  error: null,
});

export const useDiagnostics = create<DiagnosticsState>(initial);

const set = useDiagnostics.setState;
const get = useDiagnostics.getState;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function resetDiagnostics() {
  stopLive();
  for (const r of Object.values(get().busy)) if (r != null) void commands.cancelRequest(r);
  set({ ...initial(), rules: get().rules });
  setTraceHeat(null, {});
}

export const setTab = (tab: DiagTab) => set({ tab });
export const setTraceSeconds = (traceSeconds: number) => set({ traceSeconds });
export const setDumpType = (dumpType: DumpType) => set({ dumpType });
export const selectSeries = (selectedSeries: string | null) => set({ selectedSeries });
export const setBaseline = (baseline: HeapSnapshot | null) => set({ baseline });
export const setTraceView = (traceView: TraceView) => set({ traceView });
export const selectTraceMethod = (traceMethod: string | null) => set({ traceMethod });

/** Shows `trace` in the CPU Trace tab and its times in the editors. */
export function showTrace(trace: TraceProfile) {
  set({ trace, traceMethod: null });
  setTraceHeat(trace.path, heatByFile(trace), lineHeatByFile(trace));
}

/** Shows the CPU Trace tab with the callers and callees of `frame` (from an editor lens). */
export function revealTraceFrame(frame: string) {
  set({ tab: "trace", traceMethod: frame, traceView: get().traceView === "flame" ? "flame" : "methods" });
}

// ---------------------------------------------------------------- tools and processes

export async function loadTools(): Promise<DiagToolStatus[]> {
  if (!isTauri()) return [];
  const tools = await call(commands.diagTools()).catch(() => [] as DiagToolStatus[]);
  set({ tools });
  return tools;
}

const toolStatus = (tool: DiagTool) => get().tools?.find((t) => t.tool === tool);

/** Runs `dotnet tool install` for the tool in a terminal tab. */
export function installTool(tool: DiagToolStatus) {
  const command = tool.command;
  void executeAction("terminal.runCommand", {
    program: tool.install.command,
    args: tool.install.args,
    name: tx`Install ${command}`,
  });
}

/** The tool's path, or a notification offering to install it. */
async function requireTool(tool: DiagTool): Promise<string | null> {
  const status = toolStatus(tool) ?? (await loadTools()).find((t) => t.tool === tool);
  if (status?.path) return status.path;
  if (status) {
    const command = status.command;
    const line = [status.install.command, ...status.install.args].join(" ");
    notifications().notify({
      group: "dotnet",
      level: "warning",
      key: `diag-missing-${tool}`,
      title: tx`${command} is not installed`,
      body: tx`Install it with: ${line}`,
      actions: [{ label: tx`Install`, run: () => installTool(status) }],
    });
  }
  return null;
}

export async function loadProcesses() {
  if (!isTauri()) return;
  set({ loadingProcesses: true });
  try {
    const processes = await call(commands.diagProcesses());
    const pid = get().pid;
    // Keep the selection while the process runs; else one started from this project, else the first.
    const root = useProject.getState().project?.root ?? null;
    const own = processes.find((p) => processGroup(p, root) === "project");
    set({
      processes,
      loadingProcesses: false,
      pid: pid != null && processes.some((p) => p.pid === pid) ? pid : ((own ?? processes[0])?.pid ?? null),
    });
  } catch (e) {
    set({ loadingProcesses: false, error: message(e) });
  }
}

export function selectProcess(pid: number | null) {
  if (pid === get().pid) return;
  stopLive();
  set({ pid, series: {}, selectedSeries: null, fired: {}, counters: null });
}

// ---------------------------------------------------------------- live counters

export async function startLive() {
  const pid = get().pid;
  if (pid == null || !isTauri() || get().live != null) return;
  if (!(await requireTool("monitor"))) return;
  const request = nextRequestId();
  const channel = new Channel<MetricSample[]>();
  channel.onmessage = (batch) => {
    if (get().live !== request) return;
    const series = addSamples(get().series, batch);
    set({ series });
    checkRules(series);
  };
  set({ live: request, livePid: pid, error: null });
  try {
    await call(commands.diagLiveMetrics(request, pid, channel));
  } catch (e) {
    if (!isCancelled(e) && get().live === request) set({ error: message(e) });
  } finally {
    if (get().live === request) set({ live: null });
  }
}

export function stopLive() {
  const request = get().live;
  if (request == null) return;
  set({ live: null });
  void commands.cancelRequest(request);
}

// ---------------------------------------------------------------- collection

async function job<T>(name: Job, tool: DiagTool, run: (request: RequestId) => Promise<T>): Promise<T | null> {
  if (!isTauri() || get().busy[name] != null) return null;
  if (!(await requireTool(tool))) return null;
  const request = nextRequestId();
  set((s) => ({ busy: { ...s.busy, [name]: request }, error: null }));
  try {
    return await run(request);
  } catch (e) {
    if (!isCancelled(e)) set({ error: message(e) });
    return null;
  } finally {
    set((s) => {
      const busy = { ...s.busy };
      delete busy[name];
      return { busy };
    });
    void loadArtifacts();
  }
}

export function cancelJob(name: Job) {
  const request = get().busy[name];
  if (request != null) void commands.cancelRequest(request);
}

const selectedPid = () => get().pid;

export async function collectStacks(pid = selectedPid()) {
  if (pid == null) return null;
  const stacks = await job("stacks", "stack", (r) => call(commands.diagStacks(r, pid)));
  if (stacks) set({ stacks });
  return stacks;
}

export async function collectGcdump(pid = selectedPid()) {
  if (pid == null) return null;
  const heap = await job("gcdump", "gcdump", (r) => call(commands.diagGcdump(r, pid)));
  if (heap) set((s) => ({ heap, baseline: s.baseline?.path === heap.path ? null : s.baseline }));
  return heap;
}

export async function collectDump(pid = selectedPid(), kind = get().dumpType) {
  if (pid == null) return null;
  const path = await job("dump", "dump", (r) => call(commands.diagDump(r, pid, kind)));
  if (path) void analyzeDump(path);
  return path;
}

export async function analyzeDump(path: string) {
  const dump = await job("analyze", "dump", (r) => call(commands.diagDumpAnalyze(r, path)));
  if (dump) set({ dump });
  return dump;
}

export async function downloadSymbols(dump: string) {
  const out = await job("symbols", "symbol", (r) => call(commands.diagSymbols(r, dump)));
  if (out != null)
    notifications().notify({
      group: "dotnet",
      level: "info",
      title: tx`Symbols downloaded`,
      body: out.trim().split("\n").slice(-3).join("\n"),
    });
}

export async function collectTrace(pid = selectedPid(), seconds = get().traceSeconds) {
  if (pid == null) return null;
  const trace = await job("trace", "trace", (r) => call(commands.diagTrace(r, pid, seconds)));
  if (trace) showTrace(trace);
  return trace;
}

export async function recordCounters(pid = selectedPid(), seconds = 10) {
  if (pid == null) return null;
  const counters = await job("counters", "counters", (r) => call(commands.diagCounters(r, pid, seconds)));
  if (counters) showRecording(counters);
  return counters;
}

/** Shows a counter recording in the Live tab in place of the live counters. */
function showRecording(counters: CounterRecording) {
  stopLive();
  set({ counters, series: addSamples({}, counters.samples, Number.MAX_SAFE_INTEGER), selectedSeries: null });
}

/** Back to the live counters from a recording. */
export function closeRecording() {
  set({ counters: null, series: {}, selectedSeries: null });
  void startLive();
}

// ---------------------------------------------------------------- interactive shells

/** The SOS shell of `dotnet-dump analyze` on a dump, in a terminal tab. */
export async function openSosConsole(dump: string) {
  const exe = await requireTool("dump");
  if (exe) void executeAction("terminal.runCommand", { program: exe, args: ["analyze", dump], name: "SOS" });
}

/** `dotnet-debug attach`: an SOS shell on the live process, in a terminal tab. */
export async function attachSosShell(pid = selectedPid()) {
  if (pid == null) return;
  const exe = await requireTool("debug");
  if (exe)
    void executeAction("terminal.runCommand", {
      program: exe,
      args: ["attach", String(pid)],
      name: `SOS ${pid}`,
    });
}

/** `dotnet-sos install`: loads SOS into LLDB, in a terminal tab. */
export async function installSosIntoLldb() {
  const exe = await requireTool("sos");
  if (exe) void executeAction("terminal.runCommand", { program: exe, args: ["install"], name: "dotnet-sos" });
}

/** Opens a `.http` file of dotnet-monitor's endpoints for the process. */
export async function openMonitorHttp(pid = selectedPid()) {
  if (pid == null || !(await requireTool("monitor"))) return;
  try {
    const path = await call(commands.diagMonitorHttp(pid));
    await maybeEditorService()?.openFile(path, { focus: true });
  } catch (e) {
    set({ error: message(e) });
  }
}

// ---------------------------------------------------------------- collected files

export async function loadArtifacts() {
  if (!isTauri()) return;
  const artifacts = await call(commands.diagArtifacts()).catch(() => null);
  if (artifacts) set({ artifacts });
}

/** Shows a collected file in its tab. */
export async function openArtifact(a: DiagArtifact) {
  const read = async <T>(f: () => Promise<T>) => {
    try {
      return await f();
    } catch (e) {
      set({ error: message(e) });
      return null;
    }
  };
  switch (a.kind) {
    case "dump":
      set({ tab: "dumps" });
      await analyzeDump(a.path);
      return;
    case "gcdump": {
      set({ tab: "memory" });
      const heap = await job("gcdump", "gcdump", (r) => call(commands.diagGcdumpRead(r, a.path)));
      if (heap) set({ heap });
      return;
    }
    case "trace": {
      set({ tab: "trace" });
      const trace = await read(() => call(commands.diagTraceRead(a.path)));
      if (trace) showTrace(trace);
      return;
    }
    case "counters": {
      set({ tab: "live" });
      const counters = await read(() => call(commands.diagCountersRead(a.path)));
      if (counters) showRecording(counters);
      return;
    }
    case "stacks": {
      set({ tab: "threads" });
      const stacks = await read(() => call(commands.diagStacksRead(a.path)));
      if (stacks) set({ stacks });
      return;
    }
  }
}

export async function deleteArtifact(a: DiagArtifact) {
  try {
    await call(commands.diagArtifactDelete(a.path));
  } catch (e) {
    set({ error: message(e) });
  }
  await loadArtifacts();
}

/** A gcdump of the list, as the baseline to compare the current snapshot with. */
export async function compareWith(a: DiagArtifact) {
  const baseline = await job("gcdump", "gcdump", (r) => call(commands.diagGcdumpRead(r, a.path)));
  if (baseline) set({ baseline });
}

// ---------------------------------------------------------------- triggers

export function updateRules(rules: TriggerRule[]) {
  saveRules(rules);
  set({ rules });
}

export function addRule() {
  const id = `rule-${Date.now().toString(36)}`;
  updateRules([
    ...get().rules,
    {
      id,
      enabled: true,
      counter: "cpu-usage",
      op: ">",
      threshold: 50,
      seconds: 5,
      action: "stacks",
      cooldownSeconds: 120,
    },
  ]);
}

export function patchRule(id: string, patch: Partial<TriggerRule>) {
  updateRules(get().rules.map((r) => (r.id === id ? { ...r, ...patch } : r)));
}

export function removeRule(id: string) {
  updateRules(get().rules.filter((r) => r.id !== id));
}

const ACTIONS: Record<TriggerAction, (pid: number) => Promise<unknown>> = {
  gcdump: (pid) => collectGcdump(pid),
  dump: (pid) => collectDump(pid),
  trace: (pid) => collectTrace(pid),
  stacks: (pid) => collectStacks(pid),
};

/** Fires the rules that hold on the live counters of the streamed process. */
function checkRules(series: Record<string, Series>) {
  const pid = get().livePid;
  if (pid == null) return;
  const now = Math.max(0, ...Object.values(series).map((s) => s.points.at(-1)?.t ?? 0));
  for (const rule of get().rules) {
    if (!ruleFires(rule, series, now, get().fired[rule.id])) continue;
    set((s) => ({ fired: { ...s.fired, [rule.id]: now } }));
    const { action, counter, op, threshold } = rule;
    void ACTIONS[action](pid).then(
      (result) => recordFiring(rule, now, result ? tx`collected` : tx`not collected`),
      (e: unknown) => recordFiring(rule, now, message(e)),
    );
    notifications().notify({
      group: "dotnet",
      level: "info",
      title: tx`Trigger fired: ${counter} ${op} ${threshold}`,
      body: tx`Collecting ${action} from process ${pid}.`,
    });
  }
}

function recordFiring(rule: TriggerRule, time: number, result: string) {
  set((s) => ({ firings: [{ rule: rule.id, time, action: rule.action, result }, ...s.firings].slice(0, 50) }));
}
