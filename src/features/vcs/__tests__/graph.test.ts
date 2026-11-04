import { describe, expect, it } from "vitest";
import { computeGraph } from "../graph";

const c = (id: string, ...parents: string[]) => ({ id, parents });

describe("computeGraph", () => {
  it("keeps a linear history in lane 0", () => {
    const { rows } = computeGraph([c("c", "b"), c("b", "a"), c("a")]);
    expect(rows.map((r) => r.lane)).toEqual([0, 0, 0]);
    expect(rows[0]!.up).toEqual([]);
    expect(rows[0]!.down).toEqual([{ from: 0, to: 0, color: 0 }]);
    expect(rows[2]!.down).toEqual([]);
    expect(new Set(rows.map((r) => r.color)).size).toBe(1);
  });

  it("opens a lane for a merge's second parent and joins it back", () => {
    // m merges f into main; f branched from a.
    const { rows } = computeGraph([c("m", "b", "f"), c("f", "a"), c("b", "a"), c("a")]);
    const [m, f, b, a] = rows;
    expect(m!.lane).toBe(0);
    expect(m!.down).toEqual([
      { from: 0, to: 0, color: m!.color },
      { from: 0, to: 1, color: f!.color },
    ]);
    expect(f!.lane).toBe(1);
    expect(f!.color).not.toBe(m!.color);
    // b passes lane 1 (f's parent a) through while sitting in lane 0.
    expect(b!.lane).toBe(0);
    expect(b!.down).toContainEqual({ from: 1, to: 1, color: f!.color });
    // a is expected by both lanes: both lines converge into its node.
    expect(a!.lane).toBe(0);
    expect(a!.up).toEqual([
      { from: 0, to: 0, color: m!.color },
      { from: 1, to: 0, color: f!.color },
    ]);
    expect(a!.width).toBe(2);
  });

  it("reuses freed lanes and continues across pages", () => {
    const first = computeGraph([c("t1", "x"), c("t2", "y")]);
    expect(first.rows.map((r) => r.lane)).toEqual([0, 1]);
    const next = computeGraph([c("x"), c("y"), c("z")], first.state);
    expect(next.rows.map((r) => r.lane)).toEqual([0, 1, 0]);
    expect(next.state.lanes).toEqual([]);
  });

  it("converges lanes that lead to the same parent at its node", () => {
    const { rows } = computeGraph([c("t1", "p"), c("t2", "p"), c("p")]);
    expect(rows[1]!.lane).toBe(1);
    expect(rows[1]!.down).toContainEqual({ from: 1, to: 1, color: rows[1]!.color });
    expect(rows[2]!.lane).toBe(0);
    expect(rows[2]!.up).toEqual([
      { from: 0, to: 0, color: rows[0]!.color },
      { from: 1, to: 0, color: rows[1]!.color },
    ]);
  });

  it("joins a merge parent that is already on another lane", () => {
    // m2 merges x, which lane 1 (from t) already leads to.
    const { rows } = computeGraph([c("t", "x"), c("m2", "b", "x"), c("x", "b"), c("b")]);
    const m2 = rows[1]!;
    expect(m2.down).toContainEqual({ from: m2.lane, to: 0, color: rows[0]!.color });
  });
});
