/**
 * CPU Trace tab (DIAG, PRF-2): samples a process's stacks with dotnet-trace and
 * shows where its threads spend their time as a flame graph, a call tree or a
 * method list. Selecting a method shows its callers and callees; methods of the
 * solution's own code open in the editor, which also shows their times.
 */
import { useCallback, useMemo, useState, type ReactNode } from "react";
import clsx from "clsx";
import type { FlameNode, FrameSource, TraceProfile } from "@/bindings";
import { tx } from "@/platform/i18n";
import { maybeEditorService } from "@/workbench/contributions";
import { Button, Checkbox, EmptyState, Icon, IconButton, SearchField, Select, T, Tabs, Tree } from "@/ui";
import { FlameGraph, FlameLegend } from "./charts";
import { setTraceHeatEnabled, useTraceHeat } from "./heat";
import { frameCategory, nodeAt, type FrameCategory } from "./model";
import { Busy, Figure, useUserModules } from "./parts";
import {
  collectTrace,
  selectTraceMethod,
  setTraceSeconds,
  setTraceView,
  useDiagnostics,
  type TraceView,
} from "./store";
import {
  butterfly,
  displayFrame,
  formatMs,
  formatShare,
  initialCallPath,
  methodTable,
  ROOT_CALLER,
  type MethodRow,
  type Neighbour,
} from "./trace";
import styles from "./Diagnostics.module.css";
import t from "./Trace.module.css";

const TRACE_SECONDS = [5, 10, 30, 60];
const secondsLabel = (seconds: number) => tx`${seconds} seconds`;
const samplingLabel = (seconds: number) => tx`Sampling for ${seconds} s…`;

const VIEWS: { id: TraceView; label: () => string; icon: string }[] = [
  { id: "flame", label: () => tx`Flame Graph`, icon: "profile" },
  { id: "tree", label: () => tx`Call Tree`, icon: "hierarchy" },
  { id: "methods", label: () => tx`Methods`, icon: "list" },
];

/** Frame name → its declaration in the solution's sources. */
function useSources(trace: TraceProfile | null): ReadonlyMap<string, FrameSource> {
  return useMemo(() => new Map((trace?.sources ?? []).map((s) => [s.frame, s])), [trace]);
}

function openSource(source: FrameSource | undefined) {
  if (!source) return;
  void maybeEditorService()?.openFile(source.path, {
    selection: { line: source.line, col: source.col },
    focus: true,
  });
}

const basename = (p: string) => p.slice(p.lastIndexOf("/") + 1);

/** `Calculator.Spin` with its namespace and module muted after it. */
function FrameLabel({ name, category, compact }: { name: string; category?: FrameCategory; compact?: boolean }) {
  if (name === ROOT_CALLER) return <span className={t.muted}>{tx`Thread start`}</span>;
  const d = displayFrame(name);
  return (
    <span className={t.frameLabel} title={name}>
      {category && <span className={clsx(t.dot, styles[`cat-${category}`])} aria-hidden="true" />}
      <span className={t.method}>{d.method}</span>
      {!compact && (d.namespace || d.module) && (
        <span className={t.muted}>{[d.namespace, d.module].filter(Boolean).join(" · ")}</span>
      )}
    </span>
  );
}

/** A share bar: `value` of `max`, in the frame's category colour. */
function ShareBar({ value, max, category }: { value: number; max: number; category?: FrameCategory }) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  return (
    <span className={t.track} aria-hidden="true">
      <span className={clsx(t.fill, category && styles[`cat-${category}`])} style={{ width: `${pct}%` }} />
    </span>
  );
}

export function TraceTab() {
  const trace = useDiagnostics((s) => s.trace);
  const seconds = useDiagnostics((s) => s.traceSeconds);
  const view = useDiagnostics((s) => s.traceView);
  const heatOn = useTraceHeat((s) => s.enabled);
  const [query, setQuery] = useState("");
  return (
    <div className={clsx(styles.scroll, t.tab)}>
      <div className={styles.actions}>
        <Select
          value={String(seconds)}
          options={TRACE_SECONDS.map((n) => ({ value: String(n), label: secondsLabel(n) }))}
          onChange={(v) => setTraceSeconds(Number(v))}
          aria-label={tx`Trace length`}
        />
        <Button size="small" icon="profile" onClick={() => void collectTrace()}>
          <T>Record CPU Trace</T>
        </Button>
        <Busy job="trace" label={samplingLabel(seconds)} />
        {trace && (
          <>
            <span className={styles.spacer} />
            <SearchField
              value={query}
              onChange={setQuery}
              placeholder={tx`Highlight frames`}
              aria-label={tx`Highlight frames`}
            />
            <Checkbox checked={heatOn} onChange={setTraceHeatEnabled} label={<T>Show times in the editor</T>} />
          </>
        )}
      </div>
      {!trace ? (
        <EmptyState title={tx`No CPU trace`}>
          <T>
            Sample the process's stacks (dotnet-trace) to see where its threads spend their time. Your own methods open
            in the editor, which then shows how long each one took.
          </T>
        </EmptyState>
      ) : (
        <TraceBody trace={trace} view={view} query={query} />
      )}
    </div>
  );
}

function TraceBody({ trace, view, query }: { trace: TraceProfile; view: TraceView; query: string }) {
  const user = useUserModules();
  const sources = useSources(trace);
  const methods = useMemo(() => methodTable(trace.root), [trace]);
  const selected = useDiagnostics((s) => s.traceMethod);
  const total = trace.root.totalMs || 1;
  const own = useMemo(
    () => methods.reduce((sum, m) => (frameCategory(m.name, user) === "user" ? sum + m.selfMs : sum), 0),
    [methods, user],
  );
  const located = useMemo(() => new Set(trace.sources.map((s) => `${s.path}:${s.line}`)).size, [trace]);
  const category = useCallback((name: string) => frameCategory(name, user), [user]);
  return (
    <>
      <div className={styles.figures}>
        <Figure label={tx`Duration`} value={`${(trace.durationMs / 1000).toFixed(1)} s`} />
        <Figure label={tx`Threads`} value={String(trace.threads)} />
        <Figure label={tx`Thread time`} value={formatMs(trace.root.totalMs)} />
        <Figure label={tx`In your code`} value={formatShare(own / total)} />
        <Figure label={tx`Methods in source`} value={String(located)} />
        <FlameLegend />
      </div>
      <Tabs
        variant="compact"
        aria-label={tx`Trace views`}
        items={VIEWS.map((v) => ({ id: v.id, label: v.label(), icon: v.icon }))}
        activeId={view}
        onSelect={(id) => setTraceView(id as TraceView)}
        className={t.views}
      />
      <div className={clsx(t.body, selected && t.withPanel)}>
        <div className={t.main}>
          {view === "flame" && (
            <TraceFlame trace={trace} user={user} query={query} selected={selected} sources={sources} />
          )}
          {view === "tree" && (
            <CallTree trace={trace} category={category} selected={selected} sources={sources} query={query} />
          )}
          {view === "methods" && (
            <MethodsTable
              rows={methods}
              total={total}
              category={category}
              selected={selected}
              sources={sources}
              query={query}
            />
          )}
        </div>
        {selected && (
          <MethodPanel root={trace.root} method={selected} total={total} category={category} sources={sources} />
        )}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- flame graph

function TraceFlame({
  trace,
  user,
  query,
  selected,
  sources,
}: {
  trace: TraceProfile;
  user: ReadonlySet<string>;
  query: string;
  selected: string | null;
  sources: ReadonlyMap<string, FrameSource>;
}) {
  const [zoomed, setZoomed] = useState<{ path: string; zoom: string }>({ path: "", zoom: "" });
  // A new trace starts unzoomed.
  const zoom = zoomed.path === trace.path ? zoomed.zoom : "";
  const focus = nodeAt(trace.root, zoom);
  return (
    <>
      {zoom && focus && (
        <div className={styles.actions}>
          <Button
            size="small"
            variant="ghost"
            icon="arrow-left"
            onClick={() => setZoomed({ path: trace.path, zoom: "" })}
          >
            <T>Reset Zoom</T>
          </Button>
          <FrameLabel name={focus.name} />
        </div>
      )}
      <FlameGraph
        root={trace.root}
        zoom={zoom}
        onZoom={(z) => setZoomed({ path: trace.path, zoom: z })}
        userModules={user}
        query={query}
        selected={selected}
        onSelect={(name) => selectTraceMethod(name === selected ? null : name)}
        onOpen={(name) => openSource(sources.get(name))}
        hasSource={(name) => sources.has(name)}
      />
    </>
  );
}

// ---------------------------------------------------------------- call tree

interface TreeNode {
  node: FlameNode;
  key: string;
}

function CallTree({
  trace,
  category,
  selected,
  sources,
  query,
}: {
  trace: TraceProfile;
  category: (name: string) => FrameCategory;
  selected: string | null;
  sources: ReadonlyMap<string, FrameSource>;
  query: string;
}) {
  const total = trace.root.totalMs || 1;
  const wrap = (n: FlameNode, key: string): TreeNode => ({ node: n, key });
  const roots = useMemo(() => trace.root.children.map((c, i) => wrap(c, String(i))), [trace]);
  const childrenOf = (n: TreeNode) => n.node.children.map((c, i) => wrap(c, `${n.key}/${i}`));
  // The path to where your own code spends its time starts expanded.
  const initial = useMemo(() => initialCallPath(trace.root, (name) => category(name) === "user"), [trace, category]);
  const [expanded, setExpanded] = useState<{ path: string; keys: Set<string> } | null>(null);
  const keys = expanded?.path === trace.path ? expanded.keys : initial;
  const q = query.trim().toLowerCase();
  return (
    <Tree<TreeNode>
      aria-label={tx`Call tree`}
      className={t.tree}
      roots={roots}
      getKey={(n) => n.key}
      getLabel={(n) => displayFrame(n.node.name).method}
      getChildren={(n) => (n.node.children.length ? childrenOf(n) : undefined)}
      isExpandable={(n) => n.node.children.length > 0}
      expanded={keys}
      onExpandedChange={(next) => setExpanded({ path: trace.path, keys: next })}
      onSelectionChange={(_, lead) => lead && selectTraceMethod(lead.node.name)}
      onActivate={(n) => openSource(sources.get(n.node.name))}
      activateExpandable={(n) => sources.has(n.node.name)}
      rowClassName={(n) =>
        clsx(n.node.name === selected && t.sameMethod, q && !n.node.name.toLowerCase().includes(q) && t.dim)
      }
      speedSearch
      renderNode={(n) => (
        <span className={t.treeRow}>
          <span className={t.share}>{formatShare(n.node.totalMs / total)}</span>
          <ShareBar value={n.node.totalMs} max={total} category={category(n.node.name)} />
          <FrameLabel name={n.node.name} category={category(n.node.name)} compact />
          {sources.has(n.node.name) && <Icon name="edit" size={12} className={t.sourceIcon} />}
          <span className={t.weight}>
            {formatMs(n.node.totalMs)}
            {n.node.selfMs > 0 && (
              <span className={t.muted}>
                {" "}
                · {tx`self`} {formatMs(n.node.selfMs)}
              </span>
            )}
          </span>
        </span>
      )}
    />
  );
}

// ---------------------------------------------------------------- methods

type MethodSort = "self" | "total" | "name";
const MAX_ROWS = 500;

function MethodsTable({
  rows,
  total,
  category,
  selected,
  sources,
  query,
}: {
  rows: readonly MethodRow[];
  total: number;
  category: (name: string) => FrameCategory;
  selected: string | null;
  sources: ReadonlyMap<string, FrameSource>;
  query: string;
}) {
  const [sort, setSort] = useState<MethodSort>("self");
  const [ownOnly, setOwnOnly] = useState(false);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = rows.filter(
      (r) => (!q || r.name.toLowerCase().includes(q)) && (!ownOnly || category(r.name) === "user"),
    );
    if (sort === "total") list.sort((a, b) => b.totalMs - a.totalMs);
    else if (sort === "name")
      list.sort((a, b) => displayFrame(a.name).method.localeCompare(displayFrame(b.name).method));
    return list;
  }, [rows, query, sort, ownOnly, category]);
  const maxSelf = Math.max(1, ...shown.slice(0, MAX_ROWS).map((r) => r.selfMs));
  const header = (key: MethodSort, label: string, numeric = false) => (
    <th
      className={clsx(numeric && styles.num)}
      aria-sort={sort === key ? (key === "name" ? "ascending" : "descending") : "none"}
    >
      <button type="button" className={t.sortButton} onClick={() => setSort(key)}>
        {label}
        {sort === key && <Icon name={key === "name" ? "arrow-up" : "arrow-down"} size={10} />}
      </button>
    </th>
  );
  return (
    <>
      <div className={styles.actions}>
        <Checkbox checked={ownOnly} onChange={setOwnOnly} label={<T>Only your code</T>} />
        <span className={t.muted}>{tx`Double-click a method of your code to open it.`}</span>
      </div>
      <table className={clsx(styles.table, t.methods)}>
        <thead>
          <tr>
            {header("name", tx`Method`)}
            {header("self", tx`Self`, true)}
            <th className={styles.barCol} aria-label={tx`Self time share`} />
            {header("total", tx`Total`, true)}
            <th className={styles.num}>
              <T>Share</T>
            </th>
          </tr>
        </thead>
        <tbody>
          {shown.slice(0, MAX_ROWS).map((r) => {
            const source = sources.get(r.name);
            return (
              <tr
                key={r.name}
                className={clsx(styles.clickable, r.name === selected && styles.rowSelected)}
                onClick={() => selectTraceMethod(r.name)}
                onDoubleClick={() => openSource(source)}
                aria-selected={r.name === selected}
              >
                <td className={t.methodCell}>
                  <FrameLabel name={r.name} category={category(r.name)} />
                  {source && (
                    <IconButton
                      icon="edit"
                      size="small"
                      title={tx`Open ${basename(source.path)}`}
                      className={t.inlineAction}
                      onClick={(e) => {
                        e.stopPropagation();
                        openSource(source);
                      }}
                    />
                  )}
                </td>
                <td className={styles.num}>{formatMs(r.selfMs)}</td>
                <td className={styles.barCol}>
                  <ShareBar value={r.selfMs} max={maxSelf} category={category(r.name)} />
                </td>
                <td className={styles.num}>{formatMs(r.totalMs)}</td>
                <td className={clsx(styles.num, t.muted)}>{formatShare(r.totalMs / total)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {shown.length > MAX_ROWS && (
        <p className={t.muted}>{tx`… ${shown.length - MAX_ROWS} more; filter to narrow down.`}</p>
      )}
    </>
  );
}

// ---------------------------------------------------------------- method panel

function MethodPanel({
  root,
  method,
  total,
  category,
  sources,
}: {
  root: FlameNode;
  method: string;
  total: number;
  category: (name: string) => FrameCategory;
  sources: ReadonlyMap<string, FrameSource>;
}) {
  const b = useMemo(() => butterfly(root, method), [root, method]);
  const d = displayFrame(method);
  const source = sources.get(method);
  const cat = category(method);
  return (
    <aside className={t.panel} aria-label={tx`Method details`}>
      <header className={t.panelHead}>
        <span className={clsx(t.dot, styles[`cat-${cat}`])} aria-hidden="true" />
        <div className={t.panelTitle}>
          <span className={t.panelMethod} title={method}>
            {d.method}
          </span>
          <span className={t.muted}>{[d.namespace, d.module].filter(Boolean).join(" · ") || tx`Native code`}</span>
        </div>
        <IconButton icon="close" size="small" title={tx`Close`} onClick={() => selectTraceMethod(null)} />
      </header>
      {source ? (
        <Button size="small" icon="edit" className={t.openButton} onClick={() => openSource(source)}>
          {tx`Open ${basename(source.path)}:${source.line + 1}`}
        </Button>
      ) : (
        cat === "user" && <p className={t.note}>{tx`Not found in the solution's C# files.`}</p>
      )}
      <dl className={t.stats}>
        <Stat label={tx`Total`} ms={b.totalMs} share={b.totalMs / total} category={cat} />
        <Stat label={tx`Self`} ms={b.selfMs} share={b.selfMs / total} category={cat} />
      </dl>
      <Neighbours
        title={tx`Called from`}
        empty={tx`Nothing calls it in this trace.`}
        items={b.callers}
        max={b.totalMs}
        category={category}
        sources={sources}
      />
      <Neighbours
        title={tx`Calls`}
        empty={tx`It calls nothing that was sampled.`}
        items={b.callees}
        max={b.totalMs}
        category={category}
        sources={sources}
        footer={
          b.selfMs > 0 ? (
            <li className={t.neighbourSelf}>
              <span className={t.muted}>{tx`In the method itself`}</span>
              <span className={t.weight}>{formatMs(b.selfMs)}</span>
            </li>
          ) : null
        }
      />
    </aside>
  );
}

function Stat({ label, ms, share, category }: { label: string; ms: number; share: number; category: FrameCategory }) {
  return (
    <div className={t.stat}>
      <dt className={t.muted}>{label}</dt>
      <dd>
        <span className={t.statValue}>{formatMs(ms)}</span>
        <span className={t.muted}>{formatShare(share)}</span>
      </dd>
      <ShareBar value={share} max={1} category={category} />
    </div>
  );
}

function Neighbours({
  title,
  empty,
  items,
  max,
  category,
  sources,
  footer,
}: {
  title: string;
  empty: string;
  items: readonly Neighbour[];
  max: number;
  category: (name: string) => FrameCategory;
  sources: ReadonlyMap<string, FrameSource>;
  footer?: ReactNode;
}) {
  return (
    <section className={t.neighbours}>
      <h4 className={t.panelHeading}>{title}</h4>
      {items.length === 0 && !footer ? (
        <p className={t.note}>{empty}</p>
      ) : (
        <ul className={t.neighbourList}>
          {items.slice(0, 30).map((n) => (
            <li key={n.name}>
              <button
                type="button"
                className={t.neighbour}
                disabled={n.name === ROOT_CALLER}
                onClick={() => selectTraceMethod(n.name)}
                onDoubleClick={() => openSource(sources.get(n.name))}
                title={n.name || undefined}
              >
                <ShareBar value={n.ms} max={max} category={n.name ? category(n.name) : undefined} />
                <FrameLabel name={n.name} category={n.name ? category(n.name) : undefined} compact />
                <span className={t.weight}>{formatMs(n.ms)}</span>
              </button>
            </li>
          ))}
          {footer}
        </ul>
      )}
    </section>
  );
}
