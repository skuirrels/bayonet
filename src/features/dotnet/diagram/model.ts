/**
 * Project dependency diagram (CI-13): the solution's projects and their project
 * references, laid out in layers. A project sits above everything it references,
 * so the projects nothing references (apps, tests) are on top and the core
 * libraries at the bottom.
 */

export interface GraphNode {
  id: string;
  label: string;
}

export interface GraphEdge {
  /** The referencing project. */
  from: string;
  /** The referenced one. */
  to: string;
}

export interface PlacedNode extends GraphNode {
  x: number;
  y: number;
  width: number;
  /** 0 for projects that reference no other project of the solution. */
  layer: number;
}

export interface PlacedEdge extends GraphEdge {
  /** SVG path from the bottom of `from` to the top of `to`. */
  path: string;
  /** Part of a reference cycle. */
  cycle: boolean;
}

export interface Layout {
  nodes: PlacedNode[];
  edges: PlacedEdge[];
  width: number;
  height: number;
}

export const NODE_HEIGHT = 32;
const ROW_GAP = 72;
const COL_GAP = 24;
const MARGIN = 24;
const CHAR_WIDTH = 7;

const nodeWidth = (label: string) => Math.max(120, Math.round(label.length * CHAR_WIDTH + 40));

/** Layer of each node: one above the highest project it references. Cycles count once. */
export function layers(nodes: readonly GraphNode[], edges: readonly GraphEdge[]): Map<string, number> {
  const out = new Map<string, number>();
  const refs = new Map<string, string[]>();
  for (const e of edges) refs.set(e.from, [...(refs.get(e.from) ?? []), e.to]);
  const visiting = new Set<string>();
  const visit = (id: string): number => {
    const known = out.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return -1;
    visiting.add(id);
    let layer = 0;
    for (const to of refs.get(id) ?? []) layer = Math.max(layer, visit(to) + 1);
    visiting.delete(id);
    out.set(id, layer);
    return layer;
  };
  for (const n of nodes) visit(n.id);
  return out;
}

/** Edges on a cycle: `to` reaches `from` again. */
function cycleEdges(edges: readonly GraphEdge[]): Set<GraphEdge> {
  const refs = new Map<string, string[]>();
  for (const e of edges) refs.set(e.from, [...(refs.get(e.from) ?? []), e.to]);
  const reaches = (start: string, target: string) => {
    const seen = new Set<string>();
    const stack = [start];
    while (stack.length) {
      const id = stack.pop()!;
      if (id === target) return true;
      if (seen.has(id)) continue;
      seen.add(id);
      stack.push(...(refs.get(id) ?? []));
    }
    return false;
  };
  return new Set(edges.filter((e) => reaches(e.to, e.from)));
}

export function layout(nodes: readonly GraphNode[], allEdges: readonly GraphEdge[]): Layout {
  const ids = new Set(nodes.map((n) => n.id));
  const edges = allEdges.filter((e) => ids.has(e.from) && ids.has(e.to) && e.from !== e.to);
  const layerOf = layers(nodes, edges);
  const top = Math.max(0, ...layerOf.values());
  const rows: GraphNode[][] = Array.from({ length: top + 1 }, () => []);
  for (const n of [...nodes].sort((a, b) => a.label.localeCompare(b.label))) rows[top - layerOf.get(n.id)!]!.push(n);

  // Barycentre ordering: each row follows the order of what it is connected to in the row above.
  const order = new Map<string, number>();
  rows.forEach((row) => row.forEach((n, i) => order.set(n.id, i)));
  const neighbours = new Map<string, string[]>();
  for (const e of edges) {
    neighbours.set(e.from, [...(neighbours.get(e.from) ?? []), e.to]);
    neighbours.set(e.to, [...(neighbours.get(e.to) ?? []), e.from]);
  }
  for (let pass = 0; pass < 2; pass++)
    for (let r = 1; r < rows.length; r++) {
      const above = new Set(rows[r - 1]!.map((n) => n.id));
      const score = (n: GraphNode) => {
        const near = (neighbours.get(n.id) ?? []).filter((id) => above.has(id)).map((id) => order.get(id)!);
        return near.length ? near.reduce((a, b) => a + b, 0) / near.length : order.get(n.id)!;
      };
      const scores = new Map(rows[r]!.map((n) => [n.id, score(n)]));
      rows[r]!.sort((a, b) => scores.get(a.id)! - scores.get(b.id)! || a.label.localeCompare(b.label));
      rows[r]!.forEach((n, i) => order.set(n.id, i));
    }

  const rowWidths = rows.map((row) => row.reduce((w, n) => w + nodeWidth(n.label), 0) + COL_GAP * (row.length - 1));
  const width = Math.max(0, ...rowWidths) + MARGIN * 2;
  const placed = new Map<string, PlacedNode>();
  rows.forEach((row, r) => {
    let x = MARGIN + (width - MARGIN * 2 - rowWidths[r]!) / 2;
    for (const n of row) {
      const w = nodeWidth(n.label);
      placed.set(n.id, { ...n, x, y: MARGIN + r * (NODE_HEIGHT + ROW_GAP), width: w, layer: layerOf.get(n.id)! });
      x += w + COL_GAP;
    }
  });
  const cycles = cycleEdges(edges);
  const centre = (id: string) => placed.get(id)!.x + placed.get(id)!.width / 2;
  // Arrow ends spread over the middle of a box, in the order of the boxes they come from.
  const spread = (node: PlacedNode, others: string[], other: string) => {
    if (others.length < 2) return node.x + node.width / 2;
    const sorted = [...others].sort((p, q) => centre(p) - centre(q));
    const span = node.width * 0.6;
    return node.x + (node.width - span) / 2 + (span * sorted.indexOf(other)) / (sorted.length - 1);
  };
  const outOf = new Map<string, string[]>();
  const into = new Map<string, string[]>();
  for (const e of edges) {
    outOf.set(e.from, [...(outOf.get(e.from) ?? []), e.to]);
    into.set(e.to, [...(into.get(e.to) ?? []), e.from]);
  }
  const placedEdges = edges.map((e): PlacedEdge => {
    const a = placed.get(e.from)!;
    const b = placed.get(e.to)!;
    const [x1, y1] = [spread(a, outOf.get(e.from)!, e.to), a.y + NODE_HEIGHT];
    const [x2, y2] = [spread(b, into.get(e.to)!, e.from), b.y];
    // A reference within a row (a cycle) bends below it.
    const bend = y2 > y1 ? (y2 - y1) / 2 : ROW_GAP / 2;
    const c2 = y2 > y1 ? y2 - bend : y2 - ROW_GAP / 2;
    return { ...e, path: `M${x1},${y1} C${x1},${y1 + bend} ${x2},${c2} ${x2},${y2}`, cycle: cycles.has(e) };
  });
  return {
    nodes: [...placed.values()],
    edges: placedEdges,
    width,
    height: MARGIN * 2 + rows.length * NODE_HEIGHT + (rows.length - 1) * ROW_GAP,
  };
}
