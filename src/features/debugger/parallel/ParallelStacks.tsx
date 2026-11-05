/**
 * The Parallel Stacks view of the Debug tool window (DBG-6): all threads'
 * stacks of a suspended session as one graph (see `model.ts`). The selected
 * thread's path is highlighted; clicking a frame selects that thread and frame.
 */
import { useEffect, useEffectEvent, useMemo, useState } from "react";
import clsx from "clsx";
import { commands } from "@/bindings";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import { basename } from "@/platform/project";
import { EmptyState, Spinner } from "@/ui";
import { selectFrame, selectThread, useDebug, type DebugSession } from "../state";
import { buildStacks, HEADER_HEIGHT, layoutStacks, ROW_HEIGHT, threadNames, type ThreadStack } from "./model";
import styles from "./ParallelStacks.module.css";

/** Threads fetched at most; an app with thousands would take too long. */
const MAX_THREADS = 64;

async function loadStacks(session: DebugSession): Promise<ThreadStack[]> {
  const threads = session.threads.length ? session.threads : await call(commands.debugThreads(session.id));
  const out = await Promise.all(
    threads.slice(0, MAX_THREADS).map(async (t) => ({
      threadId: t.id,
      threadName: t.name,
      frames: await call(commands.debugStackTrace(session.id, t.id)).catch(() => []),
    })),
  );
  return out.filter((s) => s.frames.length);
}

/** Selects `threadId` and its frame at `index` (innermost 0). */
async function goToFrame(sessionId: number, threadId: number, index: number) {
  const before = useDebug.getState().sessions[sessionId];
  if (before?.selectedThread !== threadId) await selectThread(sessionId, threadId);
  const frame = useDebug.getState().sessions[sessionId]?.frames[index];
  if (frame) selectFrame(sessionId, frame.id);
}

export function ParallelStacks({ session }: { session: DebugSession }) {
  // A new stop (or another thread list) means new stacks.
  const key = `${session.id}:${session.status}:${session.stoppedThread}:${session.hits}:${session.threads.length}:${session.frames[0]?.id ?? ""}`;
  const [loaded, setLoaded] = useState<{ key: string; stacks: ThreadStack[] } | null>(null);
  const paused = session.status === "paused";
  // The latest session, read when `key` says the stacks changed.
  const load = useEffectEvent(() => loadStacks(session));
  useEffect(() => {
    if (!paused) return;
    let live = true;
    void load().then((stacks) => {
      if (live) setLoaded({ key, stacks });
    });
    return () => {
      live = false;
    };
  }, [key, paused]);
  const stacks = paused && loaded?.key === key ? loaded.stacks : null;
  const layout = useMemo(() => (stacks ? layoutStacks(buildStacks(stacks)) : null), [stacks]);

  if (!paused) return <EmptyState title={tx`Parallel stacks are shown while the program is suspended`} />;
  if (!stacks || !layout) return <Spinner size={16} />;
  if (!layout.boxes.length) return <EmptyState title={tx`No threads`} />;
  const current = session.selectedThread;
  const selectedIndex = session.frames.findIndex((f) => f.id === session.selectedFrame);
  const byId = new Map(layout.boxes.map((p) => [p.box.id, p]));
  return (
    <div className={styles.canvas}>
      <div className={styles.graph} style={{ width: layout.width, height: layout.height }}>
        <svg className={styles.edges} width={layout.width} height={layout.height} aria-hidden>
          {layout.boxes.map((p) => {
            const parent = p.parent ? byId.get(p.parent) : null;
            if (!parent) return null;
            const x1 = p.x + p.width / 2;
            const y1 = p.y + p.height;
            const x2 = Math.min(Math.max(x1, parent.x + 12), parent.x + parent.width - 12);
            const y2 = parent.y;
            const mid = (y1 + y2) / 2;
            return (
              <path
                key={p.box.id}
                d={`M${x1},${y1} C${x1},${mid} ${x2},${mid} ${x2},${y2}`}
                className={clsx(styles.edge, current != null && p.box.threads.includes(current) && styles.edgeCurrent)}
              />
            );
          })}
        </svg>
        {layout.boxes.map((p) => {
          const names = threadNames(p.box, stacks);
          const mine = current != null && p.box.threads.includes(current);
          const count = p.box.threads.length;
          const target = mine ? current! : p.box.threads[0]!;
          return (
            <div
              key={p.box.id}
              className={clsx(styles.box, mine && styles.current)}
              style={{ left: p.x, top: p.y, width: p.width, height: p.height }}
            >
              <div className={styles.header} style={{ height: HEADER_HEIGHT }} title={names.join("\n")}>
                {count === 1 ? names[0] : tx`${count} Threads`}
              </div>
              {[...p.box.frames].reverse().map((f, i) => {
                const index = f.indexByThread.get(target)!;
                const line = f.lines.get(target);
                const active = mine && index === selectedIndex;
                const where = f.path ? `${basename(f.path)}:${(line ?? 0) + 1}` : "";
                return (
                  <button
                    key={i}
                    type="button"
                    className={clsx(styles.frame, active && styles.active)}
                    style={{ height: ROW_HEIGHT }}
                    title={where ? `${f.name}\n${where}` : f.name}
                    onClick={() => void goToFrame(session.id, target, index)}
                  >
                    {f.name}
                  </button>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
