/**
 * MSBuild binary log viewer (BLD-3), opened for `.binlog` files: a summary of
 * the build, the project → target → task tree with each node's place on the
 * build's timeline, search over every name and message, and the details of the
 * selected node (files, durations, command lines, its messages).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import clsx from "clsx";
import type { Binlog, BinlogMessage, BinlogSeverity } from "@/bindings";
import { call, commands } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { maybeEditorService, type EditorInput } from "@/workbench/contributions";
import { copySelectionOnKeyDown } from "@/ui/selectableText";
import {
  Checkbox,
  EmptyState,
  Icon,
  SearchField,
  Spinner,
  SplitGroup,
  SplitHandle,
  SplitPanel,
  T,
  Tree,
  type TreeHandle,
} from "@/ui";
import { formatDuration } from "@/features/build/model";
import {
  ancestorKeys,
  childNodes,
  findBinlogNode,
  hasChildren,
  indexLog,
  projectContext,
  rootNodes,
  searchLog,
  slowestTargets,
  timelineBar,
  type BinlogIndex,
  type BinlogNode,
  type SearchHit,
} from "./model";
import styles from "./Binlog.module.css";

const SEVERITY_ICON: Record<BinlogSeverity, string> = {
  error: "error",
  warning: "warning",
  high: "info",
  normal: "info",
  low: "info",
};

function openAt(file: string | null, line = 0, col = 0) {
  if (!file) return;
  void maybeEditorService()
    ?.openFile(file, {
      selection: line > 0 ? { line: line - 1, col: Math.max(0, col - 1) } : undefined,
      focus: true,
    })
    .catch(() => undefined);
}

function Bar({ start, duration, total, failed }: { start: number; duration: number; total: number; failed?: boolean }) {
  const { left, width } = timelineBar(start, duration, total);
  return (
    <span className={styles.track} title={`${formatDuration(start)} + ${formatDuration(duration)}`}>
      <span
        className={clsx(styles.bar, failed && styles.barFailed)}
        style={{ left: `${left * 100}%`, width: `max(2px, ${width * 100}%)` }}
      />
    </span>
  );
}

function MessageRow({ m }: { m: BinlogMessage }) {
  return (
    <span className={styles.row} title={m.text}>
      <Icon name={SEVERITY_ICON[m.severity]} size={14} className={styles[`sev-${m.severity}`]} />
      {m.code && <span className={styles.code}>{m.code}</span>}
      <span className={clsx(styles.text, m.severity === "low" && styles.low)}>{m.text.split("\n")[0]}</span>
    </span>
  );
}

function label(n: BinlogNode): string {
  switch (n.kind) {
    case "diagnostics":
      return "Errors and Warnings";
    case "messages":
      return "Build Messages";
    case "project":
      return n.project.name;
    case "target":
      return n.target.name;
    case "task":
      return n.task.name;
    case "message":
      return n.message.text;
  }
}

function NodeRow({ n, total }: { n: BinlogNode; total: number }) {
  switch (n.kind) {
    case "diagnostics":
    case "messages":
      return (
        <span className={styles.row}>
          <Icon name={n.kind === "diagnostics" ? "warning" : "info"} size={14} />
          <span className={styles.strong}>{label(n)}</span>
          <span className={styles.muted}>{n.items.length}</span>
        </span>
      );
    case "project": {
      const p = n.project;
      const context = projectContext(p);
      return (
        <span className={styles.row}>
          <Icon
            name={/\.fsproj$/i.test(p.name) ? "project-fsharp" : /\.slnx?$/i.test(p.name) ? "module" : "project-csharp"}
            size={16}
          />
          <span className={clsx(styles.strong, !p.succeeded && styles.failed)}>{p.name}</span>
          {context && <span className={styles.muted}>{context}</span>}
          {p.entryTargets && <span className={styles.muted}>{p.entryTargets}</span>}
          <span className={styles.duration}>{formatDuration(p.durationMs)}</span>
          <Bar start={p.startMs} duration={p.durationMs} total={total} failed={!p.succeeded} />
        </span>
      );
    }
    case "target": {
      const t = n.target;
      return (
        <span className={clsx(styles.row, t.skipped && styles.skipped)}>
          <Icon name="function" size={14} />
          <span className={clsx(!t.succeeded && styles.failed)}>{t.name}</span>
          {t.skipped && <span className={styles.muted}>{t.skipped}</span>}
          {!t.skipped && <span className={styles.duration}>{formatDuration(t.durationMs)}</span>}
          {!t.skipped && <Bar start={t.startMs} duration={t.durationMs} total={total} failed={!t.succeeded} />}
        </span>
      );
    }
    case "task": {
      const k = n.task;
      return (
        <span className={styles.row}>
          <Icon name="method" size={14} />
          <span className={clsx(!k.succeeded && styles.failed)}>{k.name}</span>
          <span className={styles.duration}>{formatDuration(k.durationMs)}</span>
          <Bar start={k.startMs} duration={k.durationMs} total={total} failed={!k.succeeded} />
        </span>
      );
    }
    case "message":
      return <MessageRow m={n.message} />;
  }
}

function Prop({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <>
      <dt className={styles.propName}>{tx(name)}</dt>
      <dd className={styles.propValue}>{children}</dd>
    </>
  );
}

function FileLink({ file, line, col }: { file: string | null | undefined; line?: number; col?: number }) {
  if (!file) return null;
  return (
    <button type="button" className={styles.link} onClick={() => openAt(file, line, col)}>
      {file}
      {line ? `:${line}${col ? `:${col}` : ""}` : ""}
    </button>
  );
}

function MessageList({ items }: { items: readonly BinlogMessage[] }) {
  if (!items.length) return null;
  const more = items.length - 500;
  return (
    <div className={styles.messages}>
      {items.slice(0, 500).map((m, i) => (
        <div key={i} className={clsx(styles.message, styles[`sev-${m.severity}`])}>
          {m.code && <span className={styles.code}>{m.code} </span>}
          <span className={styles.messageText}>{m.text}</span>
          {m.file && (
            <>
              {" "}
              <FileLink file={m.file} line={m.line} col={m.col} />
            </>
          )}
        </div>
      ))}
      {items.length > 500 && <div className={styles.muted}>{tx`… ${more} more`}</div>}
    </div>
  );
}

function Summary({ log }: { log: Binlog }) {
  const slow = useMemo(() => slowestTargets(log), [log]);
  const errors = log.diagnostics.filter((d) => d.severity === "error").length;
  const warnings = log.diagnostics.length - errors;
  const top = slow[0]?.target.durationMs ?? 0;
  return (
    <div className={styles.detail}>
      <dl className={styles.props}>
        <Prop name="Result">
          <span className={log.succeeded ? styles.ok : styles.failed}>
            {log.cancelled ? tx`Cancelled` : log.succeeded ? tx`Succeeded` : tx`Failed`}
          </span>
        </Prop>
        <Prop name="Duration">{formatDuration(log.durationMs)}</Prop>
        <Prop name="Project builds">{log.projects.length}</Prop>
        <Prop name="Errors">{errors}</Prop>
        <Prop name="Warnings">{warnings}</Prop>
        <Prop name="Format">{log.formatVersion}</Prop>
      </dl>
      {slow.length > 0 && (
        <>
          <h3 className={styles.heading}>
            <T>Slowest targets</T>
          </h3>
          <div className={styles.chart}>
            {slow.map(({ project, target }, i) => (
              <div key={i} className={styles.chartRow} title={`${project} › ${target.name}`}>
                <span className={styles.chartLabel}>
                  {target.name} <span className={styles.muted}>{project}</span>
                </span>
                <span className={styles.chartTrack}>
                  <span
                    className={styles.chartBar}
                    style={{ width: `${top ? (target.durationMs / top) * 100 : 0}%` }}
                  />
                </span>
                <span className={styles.duration}>{formatDuration(target.durationMs)}</span>
              </div>
            ))}
          </div>
        </>
      )}
      {log.truncated && (
        <p className={styles.muted}>
          <T>Some low-importance messages were left out of this view.</T>
        </p>
      )}
    </div>
  );
}

function Detail({ ix, node }: { ix: BinlogIndex; node: BinlogNode | undefined }) {
  if (!node) return <Summary log={ix.log} />;
  switch (node.kind) {
    case "diagnostics":
    case "messages":
      return (
        <div className={styles.detail}>
          <MessageList items={node.items} />
        </div>
      );
    case "project": {
      const p = node.project;
      return (
        <div className={styles.detail}>
          <dl className={styles.props}>
            <Prop name="Project">
              <FileLink file={p.file} />
            </Prop>
            {p.targetFramework && <Prop name="Target framework">{p.targetFramework}</Prop>}
            {p.configuration && <Prop name="Configuration">{p.configuration}</Prop>}
            <Prop name="Targets">{p.entryTargets || tx`(default)`}</Prop>
            <Prop name="Result">
              <span className={p.succeeded ? styles.ok : styles.failed}>
                {p.succeeded ? tx`Succeeded` : tx`Failed`}
              </span>
            </Prop>
            <Prop name="Started">{formatDuration(p.startMs)}</Prop>
            <Prop name="Duration">{formatDuration(p.durationMs)}</Prop>
            <Prop name="Errors">{p.errors}</Prop>
            <Prop name="Warnings">{p.warnings}</Prop>
          </dl>
          <MessageList items={p.messages} />
        </div>
      );
    }
    case "target": {
      const t = node.target;
      return (
        <div className={styles.detail}>
          <dl className={styles.props}>
            <Prop name="Target">{t.name}</Prop>
            <Prop name="Project">{node.project.name}</Prop>
            {t.file && (
              <Prop name="Defined in">
                <FileLink file={t.file} />
              </Prop>
            )}
            {t.parent && <Prop name="Run for">{t.parent}</Prop>}
            {t.skipped ? (
              <Prop name="Skipped">{t.skipped}</Prop>
            ) : (
              <>
                <Prop name="Started">{formatDuration(t.startMs)}</Prop>
                <Prop name="Duration">{formatDuration(t.durationMs)}</Prop>
              </>
            )}
          </dl>
          <MessageList items={t.messages} />
        </div>
      );
    }
    case "task": {
      const k = node.task;
      return (
        <div className={styles.detail}>
          <dl className={styles.props}>
            <Prop name="Task">{k.name}</Prop>
            <Prop name="Target">{node.target.name}</Prop>
            <Prop name="Started">{formatDuration(k.startMs)}</Prop>
            <Prop name="Duration">{formatDuration(k.durationMs)}</Prop>
          </dl>
          {k.commandLine && (
            <>
              <h3 className={styles.heading}>
                <T>Command line</T>
              </h3>
              <pre className={styles.command}>{k.commandLine}</pre>
            </>
          )}
          <MessageList items={k.messages} />
        </div>
      );
    }
    case "message":
      return (
        <div className={styles.detail}>
          <MessageList items={[node.message]} />
        </div>
      );
  }
}

function Results({ hits, onPick }: { hits: readonly SearchHit[]; onPick: (key: string) => void }) {
  if (!hits.length) return <EmptyState title={tx`Nothing found`} />;
  return (
    <div className={styles.results} role="listbox" aria-label={tx("Search results")}>
      {hits.map((h) => (
        <button key={h.key} type="button" className={styles.result} onClick={() => onPick(h.key)}>
          <span className={styles.resultText}>{h.text.split("\n")[0]}</span>
          <span className={styles.muted}>{h.location}</span>
        </button>
      ))}
    </div>
  );
}

export function BinlogViewer({ input }: { input: EditorInput }) {
  const path = input.path ?? "";
  const [loaded, setLoaded] = useState<{ path: string; log?: Binlog; error?: string } | null>(null);
  const [showSkipped, setShowSkipped] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const tree = useRef<TreeHandle>(null);

  useEffect(() => {
    let live = true;
    call(commands.dotnetBinlogRead(path))
      .then((l) => {
        if (!live) return;
        setLoaded({ path, log: l });
        // Errors and the top projects open.
        const open = new Set<string>();
        if (l.diagnostics.length) open.add("diagnostics");
        for (const n of rootNodes(indexLog(l))) if (n.kind === "project") open.add(n.key);
        setExpanded(open);
      })
      .catch((e: unknown) => live && setLoaded({ path, error: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [path]);
  const current = loaded?.path === path ? loaded : null;
  const log = current?.log ?? null;
  const error = current?.error ?? null;

  const ix = useMemo(() => (log ? indexLog(log, showSkipped) : null), [log, showSkipped]);
  const roots = useMemo(() => (ix ? rootNodes(ix) : []), [ix]);
  const hits = useMemo(() => (ix && query.trim() ? searchLog(ix, query) : []), [ix, query]);
  const node = useMemo(() => (ix && selected ? findBinlogNode(ix, selected) : undefined), [ix, selected]);
  const selection = useMemo(() => new Set(selected ? [selected] : []), [selected]);

  const reveal = useCallback(
    (key: string) => {
      if (!ix) return;
      const target = findBinlogNode(ix, key);
      if (target?.kind === "target" && target.target.skipped) setShowSkipped(true);
      const parts = key.split("/");
      for (let i = 1; i < parts.length; i++) {
        const n = findBinlogNode(ix, parts.slice(0, i + 1).join("/"));
        if (n?.kind === "target" && n.target.skipped) setShowSkipped(true);
      }
      setExpanded((prev) => new Set([...prev, ...ancestorKeys(ix, key)]));
      setSelected(key);
      setQuery("");
      requestAnimationFrame(() => tree.current?.reveal(key));
    },
    [ix],
  );

  if (error)
    return (
      <div className={styles.view}>
        <EmptyState title={tx`Cannot read the binary log`}>{error}</EmptyState>
      </div>
    );
  if (!log || !ix)
    return (
      <div className={styles.view}>
        <div className={styles.loading}>
          <Spinner size={16} label={tx("Reading the binary log")} />
          <T>Reading the binary log…</T>
        </div>
      </div>
    );

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <span className={clsx(styles.status, log.succeeded ? styles.ok : styles.failed)}>
          <Icon name={log.succeeded ? "success" : "error"} size={14} />
          {log.cancelled ? tx`Build cancelled` : log.succeeded ? tx`Build succeeded` : tx`Build failed`}
        </span>
        <span className={styles.muted}>{formatDuration(log.durationMs)}</span>
        <SearchField
          value={query}
          onChange={setQuery}
          placeholder={tx("Search projects, targets, tasks and messages")}
          clearable
          className={styles.search}
          aria-label={tx("Search the binary log")}
        />
        <Checkbox checked={showSkipped} onChange={setShowSkipped} label={tx("Show skipped targets")} />
      </div>
      <SplitGroup orientation="horizontal" className={styles.split} id="binlog-split">
        <SplitPanel id="binlog-tree" defaultSize="55%" minSize="200px" className={styles.pane}>
          {query.trim() ? (
            <Results hits={hits} onPick={reveal} />
          ) : (
            <Tree<BinlogNode>
              handleRef={tree}
              aria-label={tx("Binary log")}
              roots={roots}
              getKey={(n) => n.key}
              getLabel={label}
              getChildren={(n) => childNodes(ix, n)}
              isExpandable={(n) => hasChildren(ix, n)}
              expanded={expanded}
              onExpandedChange={(next) => setExpanded(next)}
              renderNode={(n) => <NodeRow n={n} total={log.durationMs} />}
              selected={selection}
              onSelectionChange={(_s, lead) => setSelected(lead?.key ?? null)}
              onActivate={(n) => {
                if (n.kind === "message") openAt(n.message.file, n.message.line, n.message.col);
                else if (n.kind === "project") openAt(n.project.file);
                else if (n.kind === "target") openAt(n.target.file ?? null);
              }}
              multiSelect={false}
              empty={<EmptyState title={tx`The binary log has no projects`} />}
            />
          )}
        </SplitPanel>
        <SplitHandle />
        <SplitPanel id="binlog-detail" minSize="200px" className={styles.pane}>
          <div className={styles.detailHost} tabIndex={0} onKeyDown={copySelectionOnKeyDown}>
            <Detail ix={ix} node={node} />
          </div>
        </SplitPanel>
      </SplitGroup>
    </div>
  );
}
