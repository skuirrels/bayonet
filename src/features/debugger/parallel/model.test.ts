import { describe, expect, it } from "vitest";
import type { StackFrame } from "@/bindings";
import { buildStacks, layoutStacks, threadNames, type ThreadStack } from "./model";

let id = 0;
const frame = (name: string, line = 1): StackFrame => ({
  id: ++id,
  name,
  path: `/s/${name.split(".")[0]}.cs`,
  line,
  column: 0,
  presentationHint: null,
});
const thread = (threadId: number, ...names: string[]): ThreadStack => ({
  threadId,
  threadName: `Worker ${threadId}`,
  // Innermost first.
  frames: names.map((n, i) => frame(n, threadId * 10 + i)),
});

describe("parallel stacks (DBG-6)", () => {
  // Three workers started from Main; two wait in the same place.
  const stacks = [
    thread(1, "Program.Main"),
    thread(2, "Queue.Wait", "Worker.Run", "Thread.Start"),
    thread(3, "Queue.Wait", "Worker.Run", "Thread.Start"),
    thread(4, "Disk.Write", "Worker.Flush", "Worker.Run", "Thread.Start"),
  ];

  it("merges shared callers and splits where the threads diverge", () => {
    const roots = buildStacks(stacks);
    const names = (b: { frames: { name: string }[] }) => b.frames.map((f) => f.name);
    expect(roots.map(names)).toEqual([["Program.Main"], ["Thread.Start", "Worker.Run"]]);
    const workers = roots[1]!;
    expect(workers.threads).toEqual([2, 3, 4]);
    expect(workers.children.map(names)).toEqual([["Queue.Wait"], ["Worker.Flush", "Disk.Write"]]);
    expect(workers.children[0]!.threads).toEqual([2, 3]);
    // Each thread's own line and stack index are kept for navigation.
    const wait = workers.children[0]!.frames[0]!;
    expect([...wait.lines]).toEqual([
      [2, 20],
      [3, 30],
    ]);
    expect(wait.indexByThread.get(3)).toBe(0);
    expect(threadNames(workers.children[0]!, stacks)).toEqual(["Worker 2", "Worker 3"]);
  });

  it("puts callers below their callees without overlaps", () => {
    const layout = layoutStacks(buildStacks(stacks));
    const at = (name: string) => layout.boxes.find((p) => p.box.frames.some((f) => f.name === name))!;
    expect(at("Queue.Wait").y + at("Queue.Wait").height).toBeLessThan(at("Worker.Run").y);
    expect(at("Queue.Wait").parent).toBe(at("Worker.Run").box.id);
    for (const a of layout.boxes)
      for (const b of layout.boxes) {
        if (a === b) continue;
        const apart = a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y;
        expect(apart).toBe(true);
      }
    for (const p of layout.boxes) {
      expect(p.x + p.width).toBeLessThanOrEqual(layout.width);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y + p.height).toBeLessThanOrEqual(layout.height);
    }
  });
});
