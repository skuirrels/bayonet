/**
 * Unit Tests tool window: every test of the solution's test projects by project,
 * namespace, class and method, with run, stop, a failed-only filter, and the
 * selected test's result (message, expected / actual, stack trace, output).
 */
import { useEffect, useMemo, type MouseEvent } from "react";
import clsx from "clsx";
import { tx } from "@/platform/i18n";
import { editorService } from "@/workbench/contributions";
import {
  EmptyState,
  Icon,
  IconButton,
  SearchField,
  showContextMenu,
  Spinner,
  SplitGroup,
  SplitHandle,
  SplitPanel,
  T,
  Tree,
  type MenuItem,
} from "@/ui";
import { StatusIcon } from "@/features/run/TestTreeView";
import { buildTree, countTests, nodeStatus, selectionOf, stackLocations, type UtNode } from "./model";
import { cancelTests, discoverTests, loadProjects, runTests, runTestsWithCoverage, useUnitTests } from "./store";
import styles from "./UnitTestsView.module.css";

const ICONS: Record<UtNode["kind"], string> = {
  project: "module",
  namespace: "package",
  class: "symbol-class",
  method: "symbol-method",
  test: "symbol-method",
};

function find(nodes: readonly UtNode[], key: string): UtNode | null {
  for (const n of nodes) {
    if (n.key === key) return n;
    const c = find(n.children, key);
    if (c) return c;
  }
  return null;
}

function openSource(path: string | null | undefined, line: number | null | undefined) {
  if (!path) return;
  void editorService().openFile(path, {
    focus: true,
    selection: { line: Math.max(0, (line ?? 1) - 1), col: 0 },
  });
}

/** The class or method declaration of a group node: the first test's file and line. */
function sourceOf(node: UtNode): { path: string | null; line: number | null } {
  const t = node.kind === "test" ? node.test : node.children.length ? sourceOfFirst(node) : null;
  return { path: t?.file ?? null, line: t?.line ?? null };
}
function sourceOfFirst(node: UtNode) {
  let n: UtNode | undefined = node;
  while (n && n.kind !== "test") n = n.children[0];
  return n?.kind === "test" ? n.test : null;
}

export function UnitTestsView() {
  const s = useUnitTests();
  useEffect(() => {
    if (!useUnitTests.getState().projects.length) void loadProjects();
  }, []);
  const roots = useMemo(() => buildTree(s.projects, s.query, s.failedOnly), [s.projects, s.query, s.failedOnly]);
  const counts = useMemo(() => countTests(s.projects), [s.projects]);
  const lead = s.lead ? find(roots, s.lead) : null;
  const selectedNodes = () => [...s.selected].map((k) => find(roots, k)).filter((n): n is UtNode => !!n);
  const runSelected = () => {
    const nodes = selectedNodes();
    if (nodes.length) void runTests(selectionOf(nodes));
  };
  const menu = (node: UtNode | null, selection: Set<string>, e: MouseEvent) => {
    if (!node) return;
    const nodes = selection.has(node.key)
      ? [...selection].map((k) => find(roots, k)).filter((n): n is UtNode => !!n)
      : [node];
    const src = sourceOf(node);
    const items: MenuItem[] = [
      {
        label: tx`Run Selected Tests`,
        icon: "run",
        disabled: !!s.busy,
        run: () => void runTests(selectionOf(nodes)),
      },
      {
        label: tx`Run Selected Tests with Coverage`,
        icon: "run-coverage",
        disabled: !!s.busy,
        run: () => void runTestsWithCoverage(selectionOf(nodes)),
      },
      {
        label: tx`Jump to Source`,
        icon: "external-link",
        disabled: !src.path,
        run: () => openSource(src.path, src.line),
      },
    ];
    showContextMenu(e, items);
  };
  const status = s.busy === "discover" ? tx`Discovering tests…` : s.busy === "run" ? tx`Running tests…` : null;
  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <IconButton icon="gutter-run-all" title="Run All Tests" disabled={!!s.busy} onClick={() => void runTests()} />
        <IconButton
          icon="run"
          title="Run Selected Tests"
          disabled={!!s.busy || s.selected.size === 0}
          onClick={runSelected}
        />
        <IconButton
          icon="run-coverage"
          title="Run All Tests with Coverage"
          disabled={!!s.busy}
          onClick={() => void runTestsWithCoverage()}
        />
        <IconButton icon="stop" title="Stop" disabled={!s.busy} onClick={cancelTests} />
        <IconButton
          icon="refresh"
          title="Build and Discover Tests"
          disabled={!!s.busy}
          onClick={() => void discoverTests()}
        />
        <IconButton
          icon="ci-failed"
          title="Show Only Failed Tests"
          pressed={s.failedOnly}
          onClick={() => useUnitTests.setState({ failedOnly: !s.failedOnly })}
        />
        <SearchField
          className={styles.search}
          value={s.query}
          onChange={(query) => useUnitTests.setState({ query })}
          placeholder="Filter tests"
          aria-label="Filter tests"
        />
        <span className={styles.counts}>
          {status ? (
            <>
              <Spinner size={12} /> {status}
            </>
          ) : counts.total ? (
            <>
              {tx`${counts.total} tests`}
              {counts.passed > 0 && <span className={styles.passed}>{tx`${counts.passed} passed`}</span>}
              {counts.failed > 0 && <span className={styles.failed}>{tx`${counts.failed} failed`}</span>}
              {counts.skipped > 0 && <span className={styles.skipped}>{tx`${counts.skipped} skipped`}</span>}
            </>
          ) : null}
        </span>
      </div>
      {s.error && (
        <div className={styles.banner} role="alert">
          {s.error}
        </div>
      )}
      <SplitGroup orientation="horizontal" id="unit-tests-split" className={styles.split}>
        <SplitPanel id="tree" minSize={220} defaultSize="50%" className={styles.panel}>
          <Tree<UtNode>
            aria-label="Unit tests"
            roots={roots}
            getKey={(n) => n.key}
            getLabel={(n) => n.name}
            getChildren={(n) => n.children}
            isExpandable={(n) => n.children.length > 0}
            expanded={s.expanded}
            onExpandedChange={(expanded) => useUnitTests.setState({ expanded })}
            selected={s.selected}
            onSelectionChange={(selected, l) => useUnitTests.setState({ selected, lead: l?.key ?? null })}
            multiSelect
            onActivate={(n) => {
              const src = sourceOf(n);
              openSource(src.path, src.line);
            }}
            onContextMenu={menu}
            renderNode={(n) => (
              <span className={styles.node}>
                <StatusIcon status={nodeStatus(n)} />
                <Icon name={ICONS[n.kind]} size={14} className={styles.kindIcon} />
                <span className={clsx(styles.name, n.kind === "test" && styles.leaf)}>{n.name}</span>
                {n.kind === "project" && <ProjectNote path={n.project} />}
                {n.kind === "test" && n.test.durationMs != null && (
                  <span className={styles.duration}>{`${Math.round(n.test.durationMs)} ms`}</span>
                )}
              </span>
            )}
            empty={
              <EmptyState title={s.projects.length ? "No tests discovered yet" : "No test projects"}>
                {s.projects.length > 0 && (
                  <button type="button" className={styles.link} onClick={() => void discoverTests()}>
                    <T>Build and Discover Tests</T>
                  </button>
                )}
              </EmptyState>
            }
          />
        </SplitPanel>
        <SplitHandle />
        <SplitPanel id="result" minSize={220} className={styles.panel}>
          <Result node={lead} output={s.output} />
        </SplitPanel>
      </SplitGroup>
    </div>
  );
}

function ProjectNote({ path }: { path: string }) {
  const p = useUnitTests((s) => s.projects.find((x) => x.path === path));
  if (!p) return null;
  if (p.busy) return <Spinner size={12} />;
  if (p.message)
    return (
      <span className={styles.projectError} title={p.message}>
        {p.message}
      </span>
    );
  const total = p.tests.size;
  return <span className={styles.duration}>{tx`${total} tests`}</span>;
}

function Result({ node, output }: { node: UtNode | null; output: string[] }) {
  if (!node || node.kind !== "test") {
    return output.length ? (
      <pre className={styles.output} aria-label="Output">
        {output.join("\n")}
      </pre>
    ) : (
      <EmptyState title="Select a test to see its result" />
    );
  }
  const t = node.test;
  return (
    <div className={styles.result}>
      <div className={styles.resultHead}>
        <StatusIcon status={nodeStatus(node)} />
        <span className={styles.resultName}>{t.displayName}</span>
        {t.durationMs != null && <span className={styles.duration}>{`${Math.round(t.durationMs)} ms`}</span>}
        {t.file && (
          <IconButton icon="external-link" title="Jump to Source" onClick={() => openSource(t.file, t.line)} />
        )}
      </div>
      {t.message && <pre className={clsx(styles.block, styles.message)}>{t.message}</pre>}
      {(t.expected != null || t.actual != null) && (
        <dl className={styles.compare}>
          <dt>
            <T>Expected</T>
          </dt>
          <dd>{t.expected ?? ""}</dd>
          <dt>
            <T>Actual</T>
          </dt>
          <dd>{t.actual ?? ""}</dd>
        </dl>
      )}
      {t.stackTrace && (
        <pre className={styles.block} aria-label="Stack trace">
          {stackLocations(t.stackTrace).map((l, i) =>
            l.path ? (
              <button key={i} type="button" className={styles.frame} onClick={() => openSource(l.path, l.line)}>
                {l.text}
              </button>
            ) : (
              <div key={i}>{l.text}</div>
            ),
          )}
        </pre>
      )}
      {t.output && (
        <pre className={styles.block} aria-label="Output">
          {t.output}
        </pre>
      )}
      {!t.message && !t.stackTrace && !t.output && t.state === "discovered" && <EmptyState title="Not run yet" />}
    </div>
  );
}
