import { describe, expect, it } from "vitest";
import { layers, layout, NODE_HEIGHT } from "./model";

const node = (id: string) => ({ id, label: id });
const edge = (from: string, to: string) => ({ from, to });

describe("project dependency diagram (CI-13)", () => {
  const nodes = ["Api", "Core", "Data", "Tests", "Tool"].map(node);
  const edges = [edge("Api", "Data"), edge("Api", "Core"), edge("Data", "Core"), edge("Tests", "Api")];

  it("puts each project above everything it references", () => {
    const l = layers(nodes, edges);
    expect(Object.fromEntries(l)).toEqual({ Core: 0, Data: 1, Api: 2, Tests: 3, Tool: 0 });
  });

  it("lays out rows top down with edges from the bottom of a project to the top of its reference", () => {
    const g = layout(nodes, edges);
    const at = (id: string) => g.nodes.find((n) => n.id === id)!;
    expect(at("Tests").y).toBeLessThan(at("Api").y);
    expect(at("Api").y).toBeLessThan(at("Data").y);
    expect(at("Core").y).toBe(at("Tool").y);
    const start = (e: { path: string }) => e.path.slice(1).split(" ")[0]!.split(",").map(Number);
    const fromApi = g.edges.filter((e) => e.from === "Api").map(start);
    expect(fromApi.every(([, y]) => y === at("Api").y + NODE_HEIGHT)).toBe(true);
    expect(new Set(fromApi.map(([x]) => x)).size).toBe(2);
    const tests = g.edges.find((e) => e.from === "Tests")!;
    expect(start(tests)[0]).toBe(at("Tests").x + at("Tests").width / 2);
    expect(g.edges.every((e) => !e.cycle)).toBe(true);
    for (const n of g.nodes) expect(n.x + n.width).toBeLessThanOrEqual(g.width);
  });

  it("survives reference cycles and ignores references outside the solution", () => {
    const g = layout([node("A"), node("B")], [edge("A", "B"), edge("B", "A"), edge("A", "Elsewhere")]);
    expect(g.nodes).toHaveLength(2);
    expect(g.edges.map((e) => e.cycle)).toEqual([true, true]);
  });
});
