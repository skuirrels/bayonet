/**
 * Project dependency diagram (CI-13): the solution's projects as boxes and their
 * project references as arrows, referencing projects above referenced ones.
 * Clicking a project highlights what it references and what references it;
 * double-clicking opens the project file.
 */
import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { commands } from "@/bindings";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import { EmptyState, IconButton, Spinner } from "@/ui";
import { maybeEditorService, type EditorInput } from "@/workbench/contributions";
import { useSolution, type SolutionProject } from "../projects/store";
import { layout, NODE_HEIGHT, type GraphEdge } from "./model";
import styles from "./ProjectDiagram.module.css";

/** The project references of `projects`, as edges. */
async function referencesOf(projects: readonly SolutionProject[]): Promise<GraphEdge[]> {
  const lists = await Promise.all(
    projects.map((p) =>
      call(commands.dotnetProjectReferences(p.path))
        .then((refs) => refs.map((to) => ({ from: p.path, to })))
        .catch(() => [] as GraphEdge[]),
    ),
  );
  return lists.flat();
}

export function ProjectDiagram(_props: { input: EditorInput }) {
  const projects = useSolution((s) => s.projects);
  const key = useMemo(() => projects.map((p) => p.path).join("\n"), [projects]);
  // The references of the projects named by `key`; a stale key means loading.
  const [loaded, setLoaded] = useState<{ key: string; edges: GraphEdge[] } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void referencesOf(projects).then((edges) => {
      if (live) setLoaded({ key, edges });
    });
    return () => {
      live = false;
    };
  }, [projects, key]);
  const refresh = () => {
    setLoaded(null);
    void referencesOf(projects).then((edges) => setLoaded({ key, edges }));
  };

  const edges = loaded?.key === key ? loaded.edges : null;
  const graph = useMemo(
    () =>
      edges
        ? layout(
            projects.map((p) => ({ id: p.path, label: p.name })),
            edges,
          )
        : null,
    [projects, edges],
  );
  const related = useMemo(() => {
    if (!selected || !graph) return null;
    const out = new Set([selected]);
    for (const e of graph.edges) {
      if (e.from === selected) out.add(e.to);
      if (e.to === selected) out.add(e.from);
    }
    return out;
  }, [selected, graph]);

  const count = projects.length;
  const references = graph?.edges.length ?? 0;
  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <IconButton icon="refresh" title="Refresh" onClick={refresh} disabled={!graph} />
        {graph ? (
          <span className={styles.summary}>{tx`${count} projects, ${references} project references`}</span>
        ) : (
          <Spinner size={14} />
        )}
      </div>
      <div className={styles.canvas} onClick={() => setSelected(null)}>
        {graph && !graph.nodes.length && <EmptyState title="No projects in the solution" />}
        {graph && graph.nodes.length > 0 && (
          <svg
            width={graph.width}
            height={graph.height}
            viewBox={`0 0 ${graph.width} ${graph.height}`}
            role="img"
            aria-label={tx`Project dependencies`}
          >
            <defs>
              {[
                ["dep-arrow", styles.arrow],
                ["dep-arrow-lit", styles.arrowLit],
              ].map(([id, cls]) => (
                <marker
                  key={id}
                  id={id}
                  viewBox="0 0 8 8"
                  refX="7"
                  refY="4"
                  markerWidth="8"
                  markerHeight="8"
                  orient="auto"
                >
                  <path d="M0,0 L8,4 L0,8 z" className={cls} />
                </marker>
              ))}
            </defs>
            {graph.edges.map((e) => (
              <path
                key={`${e.from}\n${e.to}`}
                d={e.path}
                markerEnd={e.from === selected || e.to === selected ? "url(#dep-arrow-lit)" : "url(#dep-arrow)"}
                className={clsx(
                  styles.edge,
                  e.cycle && styles.cycle,
                  (e.from === selected || e.to === selected) && styles.lit,
                )}
              />
            ))}
            {graph.nodes.map((n) => (
              <g
                key={n.id}
                className={clsx(
                  styles.node,
                  n.id === selected && styles.selected,
                  related?.has(n.id) && styles.lit,
                  related && !related.has(n.id) && styles.dim,
                )}
                transform={`translate(${n.x},${n.y})`}
                onClick={(ev) => {
                  ev.stopPropagation();
                  setSelected(n.id === selected ? null : n.id);
                }}
                onDoubleClick={() => void maybeEditorService()?.openFile(n.id, { focus: true })}
              >
                <title>{n.id}</title>
                <rect width={n.width} height={NODE_HEIGHT} rx={6} />
                <text x={n.width / 2} y={NODE_HEIGHT / 2}>
                  {n.label}
                </text>
              </g>
            ))}
          </svg>
        )}
      </div>
    </div>
  );
}
