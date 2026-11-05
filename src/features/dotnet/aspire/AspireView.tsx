/**
 * Aspire tool window (BLD-5): run the AppHost, open its dashboard, and see its
 * resources (state, health, endpoints, commands) with the console of the
 * selected one, live while the window is open.
 */
import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import clsx from "clsx";
import type { AspireLogLine, AspireResource } from "@/bindings";
import { tx } from "@/platform/i18n";
import { openExternalUrl } from "@/platform/openUrl";
import { AnsiStream, spanStyle } from "@/ui/ansi";
import {
  Button,
  EmptyState,
  Icon,
  IconButton,
  Select,
  Spinner,
  SplitGroup,
  SplitHandle,
  SplitPanel,
  T,
  type IconName,
} from "@/ui";
import { availableCommands, resourceLabel, resourceTone, stateLabel } from "./model";
import {
  debugResource,
  installCli,
  loadCli,
  openDashboard,
  runAppHost,
  runCommand,
  selectAppHost,
  selectResource,
  startWatch,
  stopAppHost,
  stopWatch,
  useAspire,
} from "./store";
import styles from "./Aspire.module.css";

const basename = (p: string) => p.split(/[\\/]/).pop() ?? p;

const COMMAND_ICONS: Record<string, IconName> = {
  start: "run",
  stop: "stop",
  restart: "rerun",
  rebuild: "build-rebuild",
};

export function AspireView() {
  const s = useAspire();
  useEffect(() => {
    void loadCli().then(() => startWatch());
    return () => stopWatch();
  }, []);
  const options = useMemo(
    () => s.apphosts.map((p) => ({ value: p, label: basename(p).replace(/\.csproj$/, "") })),
    [s.apphosts],
  );
  const selected = s.resources.find((r) => r.name === s.resource);

  if (s.cli && !s.cli.path)
    return (
      <EmptyState
        title="The Aspire CLI is not installed"
        actions={
          <Button size="small" variant="primary" onClick={installCli}>
            {tx`Install Aspire CLI`}
          </Button>
        }
      >
        <T>It reports what a running AppHost does.</T>
      </EmptyState>
    );

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        {options.length > 1 ? (
          <Select
            className={styles.apphost}
            value={s.selected}
            options={options}
            onChange={selectAppHost}
            aria-label="AppHost"
          />
        ) : (
          <span className={styles.apphostName} title={s.selected ?? undefined}>
            {options[0]?.label}
          </span>
        )}
        <Status />
        {s.running ? (
          <IconButton icon="stop" title="Stop AppHost" onClick={() => void stopAppHost()} />
        ) : (
          <IconButton
            icon="run"
            title="Run AppHost"
            disabled={!s.known || s.starting}
            onClick={() => void runAppHost()}
          />
        )}
        <IconButton
          icon="web"
          title="Open Dashboard"
          disabled={!s.running?.dashboardUrl}
          onClick={() => openDashboard()}
        />
      </div>
      {s.error && (
        <div className={styles.banner} role="alert">
          {s.error}
        </div>
      )}
      {!s.running ? (
        <EmptyState title={s.known ? (s.starting ? "Starting the AppHost…" : "The AppHost is not running") : ""}>
          {s.known && !s.starting && <T>Run it to see its resources.</T>}
        </EmptyState>
      ) : (
        <SplitGroup orientation="horizontal" id="aspire-split" className={styles.split}>
          <SplitPanel id="resources" minSize={260} defaultSize="55%" className={styles.panel}>
            <Resources />
          </SplitPanel>
          <SplitHandle />
          <SplitPanel id="console" minSize={200} className={styles.panel}>
            {selected ? (
              <Console resource={resourceLabel(selected, s.resources)} lines={s.logs} live={s.logsRequest != null} />
            ) : (
              <EmptyState title="Select a resource to see its console" />
            )}
          </SplitPanel>
        </SplitGroup>
      )}
    </div>
  );
}

function Status() {
  const { running, known, starting } = useAspire();
  if (!known || starting) return <Spinner size={14} />;
  return (
    <span className={clsx(styles.status, running ? styles.running : styles.stopped)}>
      <span className={styles.dot} />
      {running ? tx`Running` : tx`Stopped`}
    </span>
  );
}

function Resources() {
  const { resources, resource } = useAspire();
  if (!resources.length) return <EmptyState title="Waiting for resources…" />;
  return (
    <div className={styles.table} role="table" aria-label="Resources">
      <div className={clsx(styles.tr, styles.th)} role="row">
        <span role="columnheader">
          <T>Name</T>
        </span>
        <span role="columnheader">
          <T>Type</T>
        </span>
        <span role="columnheader">
          <T>State</T>
        </span>
        <span role="columnheader">
          <T>Endpoints</T>
        </span>
        <span role="columnheader" />
      </div>
      {resources.map((r) => (
        <ResourceRow key={r.name} r={r} label={resourceLabel(r, resources)} selected={r.name === resource} />
      ))}
    </div>
  );
}

function ResourceRow({ r, label, selected }: { r: AspireResource; label: string; selected: boolean }) {
  const busy = useAspire((s) => s.busy);
  const tone = resourceTone(r);
  const state = stateLabel(r.state);
  const health = r.health && tone !== "stopped" ? ` · ${r.health}` : "";
  return (
    <div
      className={clsx(styles.tr, selected && styles.selected)}
      role="row"
      aria-selected={selected}
      tabIndex={0}
      onClick={() => selectResource(r.name)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") selectResource(r.name);
      }}
    >
      <span className={styles.name} role="cell" title={r.source ?? undefined}>
        <Icon name={r.resourceType === "Project" ? "module" : "toolwindow-services"} size={16} />
        {label}
      </span>
      <span className={styles.muted} role="cell">
        {r.resourceType}
      </span>
      <span className={clsx(styles.state, styles[tone])} role="cell">
        <span className={styles.dot} />
        {`${state}${health}`}
        {r.exitCode != null && tone === "failed" && <span className={styles.muted}>{` (${r.exitCode})`}</span>}
      </span>
      <span className={styles.urls} role="cell">
        {r.urls.map((u) => (
          <a
            key={u.url}
            href={u.url}
            title={u.name}
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              void openExternalUrl(u.url);
            }}
          >
            {u.url}
          </a>
        ))}
      </span>
      <span className={styles.actions} role="cell">
        {availableCommands(r).map((c) =>
          busy.has(`${r.name}:${c.name}`) ? (
            <Spinner key={c.name} size={14} />
          ) : (
            <IconButton
              key={c.name}
              icon={COMMAND_ICONS[c.name] ?? "more"}
              title={c.description ?? c.displayName}
              onClick={(e) => {
                e.stopPropagation();
                void runCommand(r, c.name);
              }}
            />
          ),
        )}
        {r.pid != null && r.resourceType === "Project" && tone !== "stopped" && tone !== "failed" && (
          <IconButton
            icon="debug"
            title="Attach Debugger"
            onClick={(e) => {
              e.stopPropagation();
              void debugResource(r);
            }}
          />
        )}
        {r.dashboardUrl && (
          <IconButton
            icon="external-link"
            title="Open in Dashboard"
            onClick={(e) => {
              e.stopPropagation();
              openDashboard(r.dashboardUrl);
            }}
          />
        )}
      </span>
    </div>
  );
}

function Console({ resource, lines, live }: { resource: string; lines: AspireLogLine[]; live: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const rendered = useMemo(() => {
    const stream = new AnsiStream();
    return lines.map((l) => ({ spans: stream.parse(l.content), error: l.isError }));
  }, [lines]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [rendered]);
  return (
    <div className={styles.console}>
      <div className={styles.consoleHead}>
        <span>{tx`Console: ${resource}`}</span>
        {live && <span className={styles.muted}>{tx`following`}</span>}
      </div>
      <div
        ref={ref}
        className={styles.lines}
        role="log"
        aria-label={tx`Console of ${resource}`}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {rendered.length === 0 ? (
          <div className={styles.muted}>{live ? tx`No output yet` : tx`No output`}</div>
        ) : (
          rendered.map((l, i) => (
            <div key={i} className={clsx(styles.line, l.error && styles.stderr)}>
              {l.spans.map((sp, j) => (
                <span key={j} style={spanStyle(sp)}>
                  {sp.text}
                </span>
              ))}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
