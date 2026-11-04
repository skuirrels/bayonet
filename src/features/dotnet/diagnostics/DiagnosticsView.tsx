/**
 * .NET Diagnostics tool window (DIAG): pick a running .NET process, watch its
 * counters live, capture its thread stacks, GC heap, a dump or a CPU trace, and
 * set rules that capture on their own when a counter crosses a threshold. The
 * interactive SOS shells (dotnet-dump analyze, dotnet-debug attach) open in the
 * terminal; what was collected is listed under Files.
 */
import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import type { DiagArtifact, DumpType, HeapType } from "@/bindings";
import { tx } from "@/platform/i18n";
import {
  Button,
  Checkbox,
  EmptyState,
  Icon,
  IconButton,
  Input,
  SearchField,
  Select,
  Spinner,
  T,
  Tabs,
  type SelectOption,
} from "@/ui";
import { useProject } from "@/platform/project";
import { LineChart, Sparkline } from "./charts";
import { Busy, Figure, useUserModules } from "./parts";
import { TraceTab } from "./TraceView";
import {
  artifactInfo,
  filterHeap,
  formatBytes,
  formatValue,
  groupStacks,
  headlineFrame,
  heapDiff,
  HEADLINE_COUNTERS,
  orderSeries,
  processGroup,
  PROCESS_GROUPS,
  type ProcessGroup,
  shortMethod,
  type HeapRow,
  type TriggerAction,
} from "./model";
import {
  addRule,
  analyzeDump,
  attachSosShell,
  closeRecording,
  collectDump,
  collectGcdump,
  collectStacks,
  compareWith,
  deleteArtifact,
  downloadSymbols,
  installSosIntoLldb,
  installTool,
  loadArtifacts,
  loadProcesses,
  loadTools,
  openArtifact,
  openMonitorHttp,
  openSosConsole,
  patchRule,
  recordCounters,
  removeRule,
  selectProcess,
  selectSeries,
  setBaseline,
  setDumpType,
  setTab,
  startLive,
  stopLive,
  useDiagnostics,
  type DiagTab,
} from "./store";
import styles from "./Diagnostics.module.css";

const GROUP_LABEL: Record<ProcessGroup, () => string> = {
  project: () => tx`This project`,
  apps: () => tx`Other .NET processes`,
  tooling: () => tx`Build and IDE tooling`,
};

const TABS: { id: DiagTab; label: () => string; icon: string }[] = [
  { id: "live", label: () => tx`Counters`, icon: "db-chart" },
  { id: "threads", label: () => tx`Threads`, icon: "hierarchy" },
  { id: "memory", label: () => tx`Memory`, icon: "package" },
  { id: "dumps", label: () => tx`Dumps`, icon: "debug" },
  { id: "trace", label: () => tx`CPU Trace`, icon: "profile" },
  { id: "triggers", label: () => tx`Triggers`, icon: "bulb" },
  { id: "files", label: () => tx`Files`, icon: "folder" },
  { id: "tools", label: () => tx`Tools`, icon: "settings" },
];

const basename = (p: string) => p.slice(p.lastIndexOf("/") + 1);
const when = (secs: number) => new Date(secs * 1000).toLocaleString();
const threadsSummary = (threads: number, stacks: number) => tx`${threads} threads, ${stacks} distinct stacks`;
const threadCount = (threads: number) => tx`${threads} threads`;
const compareLabel = (path: string) => {
  const file = basename(path);
  return tx`Compare with ${file}`;
};
const moreRows = (more: number) => tx`… ${more} more; filter to narrow down.`;

export function DiagnosticsView() {
  const s = useDiagnostics();
  useEffect(() => {
    void loadTools();
    void loadProcesses();
    void loadArtifacts();
  }, []);
  // Live counters stream while the Counters tab shows the selected process.
  const wantLive = s.tab === "live" && s.pid != null && !s.counters;
  useEffect(() => {
    if (wantLive) void startLive();
  }, [wantLive, s.pid]);
  const root = useProject((p) => p.project?.root ?? null);
  const options = useMemo(() => {
    const out: SelectOption[] = [];
    for (const group of PROCESS_GROUPS) {
      const members = s.processes.filter((p) => processGroup(p, root) === group);
      if (!members.length) continue;
      out.push({ value: `group:${group}`, label: GROUP_LABEL[group](), section: true });
      for (const p of members)
        out.push({
          value: String(p.pid),
          label: `${p.name} (${p.pid})`,
          detail: p.commandLine,
          keywords: p.commandLine,
        });
    }
    return out;
  }, [s.processes, root]);
  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <Select
          className={styles.process}
          value={s.pid == null ? null : String(s.pid)}
          options={options}
          onChange={(v) => selectProcess(Number(v))}
          placeholder={tx`No .NET process running`}
          aria-label={tx`Process`}
          searchable
        />
        <IconButton icon="refresh" title={tx`Refresh Processes`} onClick={() => void loadProcesses()} />
        {s.loadingProcesses && <Spinner size={14} />}
        <span className={styles.spacer} />
        {s.live != null ? (
          <span className={styles.liveBadge}>
            <span className={styles.liveDot} />
            <T>Live</T>
          </span>
        ) : null}
      </div>
      <Tabs
        variant="compact"
        items={TABS.map((t) => ({ id: t.id, label: t.label(), icon: t.icon }))}
        activeId={s.tab}
        onSelect={(id) => setTab(id as DiagTab)}
        aria-label={tx`Diagnostics`}
      />
      {s.error && (
        <div className={styles.banner} role="alert">
          <Icon name="error" size={14} />
          <span className={styles.bannerText}>{s.error}</span>
          <IconButton
            icon="close"
            title={tx`Dismiss`}
            size="small"
            onClick={() => useDiagnostics.setState({ error: null })}
          />
        </div>
      )}
      <div className={styles.body}>
        {s.pid == null && !["files", "tools", "triggers"].includes(s.tab) ? (
          <EmptyState title={tx`No .NET process selected`}>
            <T>Run a .NET application, then pick it above.</T>
          </EmptyState>
        ) : (
          <TabBody tab={s.tab} />
        )}
      </div>
    </div>
  );
}

function TabBody({ tab }: { tab: DiagTab }) {
  switch (tab) {
    case "live":
      return <LiveTab />;
    case "threads":
      return <ThreadsTab />;
    case "memory":
      return <MemoryTab />;
    case "dumps":
      return <DumpsTab />;
    case "trace":
      return <TraceTab />;
    case "triggers":
      return <TriggersTab />;
    case "files":
      return <FilesTab />;
    case "tools":
      return <ToolsTab />;
  }
}

// ---------------------------------------------------------------- counters

function LiveTab() {
  const series = useDiagnostics((s) => s.series);
  const selected = useDiagnostics((s) => s.selectedSeries);
  const live = useDiagnostics((s) => s.live);
  const recording = useDiagnostics((s) => s.counters);
  const [query, setQuery] = useState("");
  const ordered = useMemo(() => orderSeries(series), [series]);
  const tiles = ordered.filter((x) => (HEADLINE_COUNTERS as readonly string[]).includes(x.name) && !x.tags);
  const chart = series[selected ?? ""] ?? tiles[0] ?? ordered[0];
  const q = query.trim().toLowerCase();
  const rows = q
    ? ordered.filter((x) => x.displayName.toLowerCase().includes(q) || x.provider.toLowerCase().includes(q))
    : ordered;
  return (
    <div className={styles.scroll}>
      <div className={styles.actions}>
        {recording ? (
          <>
            <span className={styles.muted}>
              <T>Recording</T> {basename(recording.path)}
            </span>
            <Button size="small" onClick={closeRecording}>
              <T>Back to Live</T>
            </Button>
          </>
        ) : live != null ? (
          <Button size="small" icon="pause" onClick={stopLive}>
            <T>Pause</T>
          </Button>
        ) : (
          <Button size="small" icon="run" onClick={() => void startLive()}>
            <T>Resume</T>
          </Button>
        )}
        <Button size="small" icon="clock" onClick={() => void recordCounters()}>
          <T>Record 10 s to CSV</T>
        </Button>
        <Busy job="counters" label={tx`Recording counters…`} />
        <Button size="small" icon="file-http" onClick={() => void openMonitorHttp()}>
          <T>dotnet-monitor HTTP API</T>
        </Button>
      </div>
      {!ordered.length ? (
        <EmptyState title={live != null ? tx`Waiting for counters…` : tx`No counters`}>
          {live != null ? <Spinner size={16} /> : <T>Resume to stream the process's counters.</T>}
        </EmptyState>
      ) : (
        <>
          <div className={styles.tiles}>
            {tiles.map((t) => {
              const last = t.points.at(-1);
              return (
                <button
                  key={t.key}
                  type="button"
                  className={clsx(styles.tile, chart?.key === t.key && styles.tileSelected)}
                  onClick={() => selectSeries(t.key)}
                  aria-pressed={chart?.key === t.key}
                >
                  <span className={styles.tileLabel}>{t.displayName}</span>
                  <span className={styles.tileValue}>{last ? formatValue(last.v, t.unit) : "–"}</span>
                  <Sparkline points={t.points.slice(-60)} />
                </button>
              );
            })}
          </div>
          {chart && (
            <section className={styles.section}>
              <h3 className={styles.heading}>
                {chart.displayName}
                <span className={styles.muted}>
                  {" "}
                  {chart.provider} · {chart.counterType === "Rate" ? tx`per second` : tx`gauge`}
                </span>
              </h3>
              <LineChart series={chart} />
            </section>
          )}
          <section className={styles.section}>
            <div className={styles.sectionBar}>
              <h3 className={styles.heading}>
                <T>All counters</T>
              </h3>
              <SearchField
                value={query}
                onChange={setQuery}
                placeholder={tx`Filter counters`}
                aria-label={tx`Filter counters`}
              />
            </div>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>
                    <T>Counter</T>
                  </th>
                  <th>
                    <T>Provider</T>
                  </th>
                  <th className={styles.num}>
                    <T>Value</T>
                  </th>
                  <th>
                    <T>Last minute</T>
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => {
                  const last = r.points.at(-1);
                  return (
                    <tr
                      key={r.key}
                      className={clsx(styles.clickable, chart?.key === r.key && styles.rowSelected)}
                      onClick={() => selectSeries(r.key)}
                    >
                      <td>{r.displayName}</td>
                      <td className={styles.muted}>{r.provider}</td>
                      <td className={styles.num}>{last ? formatValue(last.v, r.unit) : "–"}</td>
                      <td className={styles.sparkCell}>
                        <Sparkline points={r.points.slice(-60)} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </section>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- threads

function ThreadsTab() {
  const stacks = useDiagnostics((s) => s.stacks);
  const user = useUserModules();
  const groups = useMemo(() => groupStacks(stacks?.threads ?? []), [stacks]);
  const [picked, setPicked] = useState<string | null>(null);
  const selected = groups.find((g) => g.key === picked) ?? groups[0];
  return (
    <div className={styles.column}>
      <div className={styles.actions}>
        <Button size="small" icon="run" onClick={() => void collectStacks()}>
          <T>Capture Stacks</T>
        </Button>
        <Busy job="stacks" label={tx`Reading stacks…`} />
        <Button size="small" icon="terminal" onClick={() => void attachSosShell()}>
          <T>SOS Shell on Live Process</T>
        </Button>
        {stacks && <span className={styles.muted}>{threadsSummary(stacks.threads.length, groups.length)}</span>}
      </div>
      {!stacks ? (
        <EmptyState title={tx`No stacks captured`}>
          <T>Capture the managed stack of every thread (dotnet-stack). Threads with the same stack are grouped.</T>
        </EmptyState>
      ) : (
        <div className={styles.split}>
          <ul className={styles.list} aria-label={tx`Stacks`}>
            {groups.map((g) => {
              const top = headlineFrame(g.frames, user);
              return (
                <li key={g.key}>
                  <button
                    type="button"
                    className={clsx(styles.listItem, g === selected && styles.listItemSelected)}
                    onClick={() => setPicked(g.key)}
                  >
                    <span className={styles.count}>{g.threads.length}</span>
                    <span className={styles.text}>{top ? shortMethod(top.method) : tx`(no frames)`}</span>
                    <span className={styles.muted}>{top?.module}</span>
                  </button>
                </li>
              );
            })}
          </ul>
          <div className={styles.detail}>
            {selected && (
              <>
                <h3 className={styles.heading}>
                  {threadCount(selected.threads.length)}
                  <span className={styles.muted}> {selected.threads.join(", ")}</span>
                </h3>
                <ol className={styles.frames}>
                  {selected.frames.map((f, i) => (
                    <li
                      key={i}
                      className={clsx(styles.frame, user.has(f.module) && styles.frameUser, !f.module && styles.muted)}
                    >
                      {f.module && <span className={styles.module}>{f.module}!</span>}
                      {f.method}
                    </li>
                  ))}
                </ol>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- memory

const HEAP_ROWS = 500;

function HeapTable({ rows, compare }: { rows: readonly HeapRow[]; compare: boolean }) {
  const max = Math.max(1, ...rows.map((r) => Math.abs(compare ? (r.deltaBytes ?? 0) : r.bytes)));
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>
            <T>Type</T>
          </th>
          <th className={styles.num}>
            <T>Count</T>
          </th>
          <th className={styles.num}>
            <T>Size</T>
          </th>
          {compare && (
            <>
              <th className={styles.num}>
                <T>Δ Count</T>
              </th>
              <th className={styles.num}>
                <T>Δ Size</T>
              </th>
            </>
          )}
          <th className={styles.barCol} aria-hidden />
        </tr>
      </thead>
      <tbody>
        {rows.slice(0, HEAP_ROWS).map((r) => {
          const v = compare ? (r.deltaBytes ?? 0) : r.bytes;
          return (
            <tr key={`${r.module}|${r.name}`} title={r.module}>
              <td className={styles.typeName}>
                {r.name}
                {r.module && <span className={styles.muted}> {r.module}</span>}
              </td>
              <td className={styles.num}>{r.count.toLocaleString()}</td>
              <td className={styles.num}>{formatBytes(r.bytes)}</td>
              {compare && (
                <>
                  <td className={styles.num}>{signed(r.deltaCount ?? 0, (n) => n.toLocaleString())}</td>
                  <td className={styles.num}>{signed(r.deltaBytes ?? 0, formatBytes)}</td>
                </>
              )}
              <td className={styles.barCol}>
                <span className={styles.barTrack}>
                  <span
                    className={clsx(styles.bar, v < 0 && styles.barShrink)}
                    style={{ width: `${(Math.abs(v) / max) * 100}%` }}
                  />
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

const signed = (n: number, f: (n: number) => string) => (n > 0 ? `+${f(n)}` : n < 0 ? `-${f(-n)}` : "0");

function MemoryTab() {
  const heap = useDiagnostics((s) => s.heap);
  const baseline = useDiagnostics((s) => s.baseline);
  const artifacts = useDiagnostics((s) => s.artifacts);
  const [query, setQuery] = useState("");
  const others = artifacts.filter((a) => a.kind === "gcdump" && a.path !== heap?.path);
  const compare = !!(heap && baseline);
  const rows = useMemo<HeapRow[]>(() => {
    if (!heap) return [];
    const all: HeapType[] = heap.report.types;
    return filterHeap(compare ? heapDiff(baseline!.report.types, all) : all, query);
  }, [heap, baseline, compare, query]);
  return (
    <div className={styles.scroll}>
      <div className={styles.actions}>
        <Button size="small" icon="run" onClick={() => void collectGcdump()}>
          <T>Capture GC Heap</T>
        </Button>
        <Busy job="gcdump" label={tx`Snapshotting the heap…`} />
        {heap && (
          <Select
            value={baseline?.path ?? ""}
            options={[
              { value: "", label: tx`Compare with… (none)` },
              ...others.map((a) => ({
                value: a.path,
                label: compareLabel(a.path),
                detail: when(a.modifiedSecs),
              })),
            ]}
            onChange={(v) => {
              const a = others.find((o) => o.path === v);
              if (a) void compareWith(a);
              else setBaseline(null);
            }}
            aria-label={tx`Baseline snapshot`}
          />
        )}
      </div>
      {!heap ? (
        <EmptyState title={tx`No heap snapshot`}>
          <T>
            Capture the live objects of the GC heap by type (dotnet-gcdump). Capture again later and compare the two to
            find what grows.
          </T>
        </EmptyState>
      ) : (
        <>
          <div className={styles.figures}>
            <Figure
              label={tx`Heap size`}
              value={formatBytes(heap.report.totalBytes)}
              delta={compare ? signed(heap.report.totalBytes - baseline!.report.totalBytes, formatBytes) : undefined}
            />
            <Figure
              label={tx`Objects`}
              value={heap.report.totalObjects.toLocaleString()}
              delta={
                compare
                  ? signed(heap.report.totalObjects - baseline!.report.totalObjects, (n) => n.toLocaleString())
                  : undefined
              }
            />
            <Figure label={tx`Types`} value={heap.report.types.length.toLocaleString()} />
            <span className={styles.muted}>{basename(heap.path)}</span>
          </div>
          <div className={styles.sectionBar}>
            <h3 className={styles.heading}>{compare ? <T>Growth by type</T> : <T>Largest types</T>}</h3>
            <SearchField
              value={query}
              onChange={setQuery}
              placeholder={tx`Filter types`}
              aria-label={tx`Filter types`}
            />
          </div>
          <HeapTable rows={rows} compare={compare} />
          {rows.length > HEAP_ROWS && <p className={styles.muted}>{moreRows(rows.length - HEAP_ROWS)}</p>}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- dumps

const DUMP_TYPES: { value: DumpType; label: () => string }[] = [
  { value: "heap", label: () => tx`With heap (for SOS)` },
  { value: "mini", label: () => tx`Mini` },
  { value: "triage", label: () => tx`Triage (no personal data)` },
  { value: "full", label: () => tx`Full` },
];

function DumpsTab() {
  const dump = useDiagnostics((s) => s.dump);
  const dumpType = useDiagnostics((s) => s.dumpType);
  const artifacts = useDiagnostics((s) => s.artifacts);
  const dumps = useMemo(() => artifacts.filter((a) => a.kind === "dump"), [artifacts]);
  const [showOutput, setShowOutput] = useState(false);
  return (
    <div className={styles.scroll}>
      <div className={styles.actions}>
        <Select
          value={dumpType}
          options={DUMP_TYPES.map((d) => ({ value: d.value, label: d.label() }))}
          onChange={(v) => setDumpType(v)}
          aria-label={tx`Dump type`}
        />
        <Button size="small" icon="run" onClick={() => void collectDump()}>
          <T>Collect Dump</T>
        </Button>
        <Busy job="dump" label={tx`Writing the dump…`} />
        <Busy job="analyze" label={tx`Analysing with SOS…`} />
        {dumps.length > 0 && (
          <Select
            value={dump?.path ?? null}
            options={dumps.map((a) => ({
              value: a.path,
              label: basename(a.path),
              detail: `${formatBytes(a.size)} · ${when(a.modifiedSecs)}`,
            }))}
            onChange={(v) => void analyzeDump(v)}
            placeholder={tx`Analyse a dump…`}
            aria-label={tx`Dump`}
          />
        )}
      </div>
      {!dump ? (
        <EmptyState title={tx`No dump analysed`}>
          <T>
            Collect a dump (dotnet-dump), or pick one collected before. Its threads and heap are summarised with SOS.
          </T>
        </EmptyState>
      ) : (
        <>
          <div className={styles.figures}>
            <Figure label={tx`Dump size`} value={formatBytes(dump.size)} />
            <Figure label={tx`Managed threads`} value={String(dump.threads.length)} />
            <Figure label={tx`Objects`} value={dump.heap.totalObjects.toLocaleString()} />
            <Figure label={tx`Heap`} value={formatBytes(dump.heap.totalBytes)} />
          </div>
          <div className={styles.actions}>
            <Button size="small" icon="terminal" onClick={() => void openSosConsole(dump.path)}>
              <T>Open SOS Console</T>
            </Button>
            <Button size="small" icon="arrow-down" onClick={() => void downloadSymbols(dump.path)}>
              <T>Download Symbols</T>
            </Button>
            <Busy job="symbols" label={tx`Downloading symbols…`} />
            <Checkbox checked={showOutput} onChange={setShowOutput} label={<T>Show SOS output</T>} />
          </div>
          {showOutput && <pre className={styles.output}>{dump.output}</pre>}
          <h3 className={styles.heading}>
            <T>Threads</T>
          </h3>
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.num}>
                  <T>ID</T>
                </th>
                <th>
                  <T>OS ID</T>
                </th>
                <th>
                  <T>Role</T>
                </th>
                <th>
                  <T>GC mode</T>
                </th>
                <th className={styles.num}>
                  <T>Locks</T>
                </th>
                <th>
                  <T>Exception</T>
                </th>
              </tr>
            </thead>
            <tbody>
              {dump.threads.map((t) => (
                <tr key={`${t.debuggerId}-${t.managedId}`} className={clsx(t.dead && styles.muted)}>
                  <td className={styles.num}>{t.managedId}</td>
                  <td className={styles.mono}>{t.osId}</td>
                  <td>{t.dead ? tx`dead` : (t.role ?? "")}</td>
                  <td>{t.gcMode}</td>
                  <td className={styles.num}>{t.lockCount < 0 ? "" : t.lockCount}</td>
                  <td className={clsx(t.exception && styles.danger)}>{t.exception ?? ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <h3 className={styles.heading}>
            <T>Heap by type</T>
          </h3>
          <HeapTable rows={dump.heap.types.slice(0, 100)} compare={false} />
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- triggers

const ACTION_LABEL: Record<TriggerAction, () => string> = {
  gcdump: () => tx`Capture GC heap`,
  dump: () => tx`Collect dump`,
  trace: () => tx`Record CPU trace`,
  stacks: () => tx`Capture stacks`,
};

function TriggersTab() {
  const rules = useDiagnostics((s) => s.rules);
  const firings = useDiagnostics((s) => s.firings);
  const series = useDiagnostics((s) => s.series);
  const live = useDiagnostics((s) => s.live);
  const counters = useMemo(() => {
    const names = new Set<string>(HEADLINE_COUNTERS);
    for (const x of Object.values(series)) if (!x.tags) names.add(x.name);
    for (const r of rules) names.add(r.counter);
    return [...names].map((n) => ({ value: n, label: n }));
  }, [series, rules]);
  const num = (v: string, min = 0) => Math.max(min, Number(v) || 0);
  return (
    <div className={styles.scroll}>
      <p className={styles.note}>
        <Icon name="info" size={14} />
        {live != null ? (
          <T>Rules are checked against the live counters of the selected process every second.</T>
        ) : (
          <T>Rules are checked while the Counters tab streams a process.</T>
        )}
      </p>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>
              <T>On</T>
            </th>
            <th>
              <T>When</T>
            </th>
            <th>
              <T>For (s)</T>
            </th>
            <th>
              <T>Then</T>
            </th>
            <th>
              <T>Wait before firing again (s)</T>
            </th>
            <th aria-hidden />
          </tr>
        </thead>
        <tbody>
          {rules.map((r) => (
            <tr key={r.id}>
              <td>
                <Checkbox
                  checked={r.enabled}
                  onChange={(enabled) => patchRule(r.id, { enabled })}
                  aria-label={tx`Enabled`}
                />
              </td>
              <td className={styles.ruleWhen}>
                <Select
                  value={r.counter}
                  options={counters}
                  onChange={(counter) => patchRule(r.id, { counter })}
                  searchable
                  aria-label={tx`Counter`}
                />
                <Select
                  value={r.op}
                  options={[
                    { value: ">", label: ">" },
                    { value: "<", label: "<" },
                  ]}
                  onChange={(op) => patchRule(r.id, { op })}
                  aria-label={tx`Comparison`}
                />
                <Input
                  className={styles.numInput}
                  type="number"
                  value={r.threshold}
                  onChange={(e) => patchRule(r.id, { threshold: Number(e.target.value) || 0 })}
                  aria-label={tx`Threshold`}
                />
              </td>
              <td>
                <Input
                  className={styles.numInput}
                  type="number"
                  min={1}
                  value={r.seconds}
                  onChange={(e) => patchRule(r.id, { seconds: num(e.target.value, 1) })}
                  aria-label={tx`Seconds`}
                />
              </td>
              <td>
                <Select
                  value={r.action}
                  options={(Object.keys(ACTION_LABEL) as TriggerAction[]).map((a) => ({
                    value: a,
                    label: ACTION_LABEL[a](),
                  }))}
                  onChange={(action) => patchRule(r.id, { action })}
                  aria-label={tx`Action`}
                />
              </td>
              <td>
                <Input
                  className={styles.numInput}
                  type="number"
                  min={0}
                  value={r.cooldownSeconds}
                  onChange={(e) => patchRule(r.id, { cooldownSeconds: num(e.target.value) })}
                  aria-label={tx`Cool-down`}
                />
              </td>
              <td>
                <IconButton icon="trash" title={tx`Remove Rule`} size="small" onClick={() => removeRule(r.id)} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className={styles.actions}>
        <Button size="small" icon="plus" onClick={addRule}>
          <T>Add Rule</T>
        </Button>
      </div>
      {firings.length > 0 && (
        <>
          <h3 className={styles.heading}>
            <T>Fired</T>
          </h3>
          <ul className={styles.firings}>
            {firings.map((f, i) => (
              <li key={i}>
                <span className={styles.mono}>{new Date(f.time).toLocaleTimeString()}</span> {ACTION_LABEL[f.action]()}{" "}
                · {rules.find((r) => r.id === f.rule)?.counter ?? f.rule} ·{" "}
                <span className={styles.muted}>{f.result}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- files and tools

const KIND_LABEL: Record<DiagArtifact["kind"], () => string> = {
  dump: () => tx`Dump`,
  gcdump: () => tx`GC heap`,
  trace: () => tx`CPU trace`,
  counters: () => tx`Counters`,
  stacks: () => tx`Stacks`,
};

function FilesTab() {
  const artifacts = useDiagnostics((s) => s.artifacts);
  return (
    <div className={styles.scroll}>
      <div className={styles.actions}>
        <IconButton icon="refresh" title={tx`Refresh`} onClick={() => void loadArtifacts()} />
      </div>
      {!artifacts.length ? (
        <EmptyState title={tx`Nothing collected yet`}>
          <T>Dumps, heap snapshots, traces, stacks and counter recordings of this project are kept here.</T>
        </EmptyState>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>
                <T>File</T>
              </th>
              <th>
                <T>Kind</T>
              </th>
              <th className={styles.num}>
                <T>Process</T>
              </th>
              <th className={styles.num}>
                <T>Size</T>
              </th>
              <th>
                <T>Collected</T>
              </th>
              <th aria-hidden />
            </tr>
          </thead>
          <tbody>
            {artifacts.map((a) => (
              <tr key={a.path} className={styles.clickable} onDoubleClick={() => void openArtifact(a)}>
                <td className={styles.typeName}>{basename(a.path)}</td>
                <td>{KIND_LABEL[a.kind]()}</td>
                <td className={styles.num}>{artifactInfo(a.path).pid ?? ""}</td>
                <td className={styles.num}>{formatBytes(a.size)}</td>
                <td>{when(a.modifiedSecs)}</td>
                <td className={styles.rowActions}>
                  <Button size="small" variant="ghost" onClick={() => void openArtifact(a)}>
                    <T>Open</T>
                  </Button>
                  <IconButton icon="trash" title={tx`Delete`} size="small" onClick={() => void deleteArtifact(a)} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ToolsTab() {
  const tools = useDiagnostics((s) => s.tools);
  return (
    <div className={styles.scroll}>
      <div className={styles.actions}>
        <IconButton icon="refresh" title={tx`Refresh`} onClick={() => void loadTools()} />
        <Button size="small" icon="terminal" onClick={() => void installSosIntoLldb()}>
          <T>Install SOS into LLDB</T>
        </Button>
      </div>
      {!tools ? (
        <Spinner size={16} />
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>
                <T>Tool</T>
              </th>
              <th>
                <T>Used for</T>
              </th>
              <th>
                <T>Status</T>
              </th>
            </tr>
          </thead>
          <tbody>
            {tools.map((t) => (
              <tr key={t.tool}>
                <td className={styles.mono}>{t.command}</td>
                <td>{t.purpose}</td>
                <td>
                  {t.path ? (
                    <span className={styles.ok} title={t.path}>
                      <Icon name="success" size={14} /> <T>Installed</T>
                    </span>
                  ) : (
                    <Button size="small" onClick={() => installTool(t)}>
                      <T>Install</T>
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
