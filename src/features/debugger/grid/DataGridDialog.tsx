/** Collection visualiser dialog: a collection's elements as a sortable table (DBG-6). */
import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { call, commands } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { Dialog, Icon } from "@/ui";
import { evaluate, useDebug } from "../state";
import { classifyValue, oneLine } from "../values";
import { useOverlays } from "../ui/popoverStore";
import { loadGrid, sortRows, type Grid } from "./model";
import styles from "./DataGridDialog.module.css";

/** Adapters debugging .NET, where `ToArray()` evaluates on most collections. */
const DOTNET_ADAPTERS = new Set(["netcoredbg", "vsdbg", "coreclr"]);

type GridTarget = NonNullable<ReturnType<typeof useOverlays.getState>["grid"]>;

export function DataGridDialog() {
  const target = useOverlays((s) => s.grid);
  const close = () => useOverlays.setState({ grid: null });
  return (
    <Dialog
      open={!!target}
      onClose={close}
      title={target?.name}
      width={720}
      resizable="both"
      aria-label="Collection visualiser"
    >
      {target && <GridBody key={`${target.session}:${target.reference}`} target={target} />}
    </Dialog>
  );
}

interface Loaded {
  /** The read this result belongs to (see `want` in `GridBody`). */
  key: string | null;
  grid: Grid | null;
  error: string | null;
}

function countText(grid: Grid) {
  const count = grid.rows.length;
  const total = count + grid.truncated;
  if (grid.truncated) return tx`First ${count} of ${total} elements`;
  return count === 1 ? tx("1 element") : tx`${count} elements`;
}

function errorText(e: unknown) {
  return e instanceof Error ? e.message : String(e);
}

function GridBody({ target }: { target: GridTarget }) {
  const stopSeq = useDebug((s) => s.stopSeq);
  const status = useDebug((s) => s.sessions[target.session]?.status);
  const frame = useDebug((s) => s.sessions[target.session]?.selectedFrame ?? null);
  const [openedAt] = useState(() => useDebug.getState().stopSeq);
  const [loaded, setLoaded] = useState<Loaded>({ key: null, grid: null, error: null });
  const [sort, setSort] = useState<{ column: string | null; descending: boolean }>({
    column: null,
    descending: false,
  });

  const sameStop = stopSeq === openedAt;
  // At a later stop the reference is gone: the collection is read again in the selected frame.
  const stale = !sameStop && !target.evaluateName;
  const canRead = status === "paused" && (sameStop || (!stale && frame != null));
  const want = sameStop ? "opened" : `${stopSeq}:${frame}`;
  useEffect(() => {
    if (!canRead) return;
    let cancelled = false;
    void (async () => {
      let reference = target.reference;
      if (!sameStop) {
        const current = useDebug.getState().sessions[target.session] ?? null;
        reference = (await evaluate(target.evaluateName!, "watch", current)).variablesReference;
        if (reference <= 0) throw new Error(tx("The value has no elements"));
      }
      const session = useDebug.getState().sessions[target.session] ?? null;
      const expression = target.evaluateName;
      // netcoredbg shows collections without type proxies as their fields: read an array copy.
      const fallback =
        expression && session && DOTNET_ADAPTERS.has(session.adapter)
          ? async () => (await evaluate(`(${expression}).ToArray()`, "watch", session)).variablesReference
          : undefined;
      const grid = await loadGrid(
        (ref) => call(commands.debugVariables(target.session, ref)),
        reference,
        () => cancelled,
        fallback,
      );
      if (!cancelled) setLoaded({ key: want, grid, error: null });
    })().catch((e) => {
      if (!cancelled) setLoaded((l) => ({ ...l, key: want, error: errorText(e) }));
    });
    return () => {
      cancelled = true;
    };
  }, [target, canRead, sameStop, want]);
  const loading = canRead && loaded.key !== want;

  const grid = loaded.grid;
  const rows = useMemo(
    () => (grid ? sortRows(grid.rows, sort.column, sort.descending) : []),
    [grid, sort.column, sort.descending],
  );
  const sortBy = (column: string | null) =>
    setSort((s) => (s.column === column ? { column, descending: !s.descending } : { column, descending: false }));

  const header = (column: string | null, label: string, className?: string) => {
    const active = sort.column === column;
    return (
      <th
        key={column ?? "\u0000index"}
        className={className}
        aria-sort={active ? (sort.descending ? "descending" : "ascending") : "none"}
      >
        <button type="button" className={styles.sortButton} onClick={() => sortBy(column)}>
          {label}
          {active && <Icon name={sort.descending ? "arrow-down" : "arrow-up"} size={10} />}
        </button>
      </th>
    );
  };

  const running = status !== "paused";
  const note = loaded.error
    ? loaded.error
    : running
      ? tx("Values are shown while the program is suspended")
      : stale
        ? tx("The values are from an earlier stop")
        : null;

  return (
    <div className={styles.root}>
      <div className={styles.status} role="status">
        {loading && !grid ? tx("Loading…") : grid ? <span>{countText(grid)}</span> : null}
        {note && <span className={clsx(styles.note, loaded.error && styles.error)}>{note}</span>}
      </div>
      <div className={clsx(styles.scroll, (running || stale) && styles.dim)}>
        {grid && grid.rows.length > 0 ? (
          <table className={styles.table}>
            <thead>
              <tr>
                {header(null, tx("Index"), styles.indexCol)}
                {grid.columns.map((c) => header(c, c))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.index}>
                  <td className={styles.indexCol}>{r.index}</td>
                  {grid.columns.map((c) => {
                    const v = r.cells.get(c);
                    if (!v) return <td key={c} />;
                    const value = oneLine(v.value);
                    return (
                      <td key={c} className={styles[classifyValue(v.value, v.typeName)]} title={value}>
                        {value}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          grid && <div className={styles.empty}>{tx("The collection is empty")}</div>
        )}
      </div>
    </div>
  );
}
