/**
 * Change Signature for C# (CI-6): the dialog's parameter rows and the checks
 * the backend makes again before it rewrites anything.
 */
import type { MethodSignature, NewParameter } from "@/bindings";
import { tx } from "@/platform/i18n";

export interface SignatureRow {
  /** Stable React key. */
  key: number;
  /** Index of the existing parameter; `null` for a new one. */
  from: number | null;
  /** The declaration, e.g. `int count = 0`. */
  text: string;
  /** What callers pass, for a new parameter. */
  value: string;
}

export function initialRows(sig: MethodSignature): SignatureRow[] {
  return sig.parameters.map((p, i) => ({ key: i, from: i, text: p.text, value: "" }));
}

export function moveRow(rows: readonly SignatureRow[], index: number, delta: -1 | 1): SignatureRow[] {
  const to = index + delta;
  if (index < 0 || to < 0 || to >= rows.length) return [...rows];
  const out = [...rows];
  [out[index], out[to]] = [out[to]!, out[index]!];
  return out;
}

/** Index of `=` outside brackets and string literals. */
function defaultAt(text: string): number {
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i]!;
    if (c === '"' || c === "'") {
      const end = text.indexOf(c, i + 1);
      if (end < 0) return -1;
      i = end;
    } else if ("([{<".includes(c)) depth++;
    else if (")]}>".includes(c)) depth--;
    else if (c === "=" && depth === 0) return i;
  }
  return -1;
}

export interface ParsedParameter {
  name: string;
  modifiers: string[];
  hasDefault: boolean;
}

const MODIFIERS = new Set(["this", "params", "ref", "out", "in", "scoped", "readonly"]);

/** Name, modifiers and default of a parameter declaration (as the backend reads it). */
export function parseParameter(text: string): ParsedParameter {
  const t = text.trim();
  const eq = defaultAt(t);
  const head = (eq < 0 ? t : t.slice(0, eq)).trim().replace(/^(\[[^\]]*\]\s*)+/, "");
  const words = head.split(/\s+/);
  const modifiers: string[] = [];
  while (words.length > 1 && MODIFIERS.has(words[0]!)) modifiers.push(words.shift()!);
  const name = /[@\w]+$/.exec(head)?.[0] ?? "";
  // A lone word is a type without a name.
  return { name: words.length > 1 || modifiers.length ? name : "", modifiers, hasDefault: eq >= 0 };
}

/** Why the rows cannot be applied, or `null`. */
export function rowsError(sig: MethodSignature, rows: readonly SignatureRow[]): string | null {
  const parsed = rows.map((r) => parseParameter(r.text));
  const names = new Set<string>();
  for (const [i, row] of rows.entries()) {
    const p = parsed[i]!;
    if (!p.name) return tx`Enter a type and a name for each parameter`;
    if (names.has(p.name)) {
      const name = p.name;
      return tx`'${name}' is declared twice`;
    }
    names.add(p.name);
    if (row.from !== null) {
      const old = sig.parameters[row.from]!.name;
      if (old !== p.name) return tx`Rename '${old}' with Rename, not Change Signature`;
    } else if (!p.hasDefault && !p.modifiers.includes("params") && !row.value.trim()) {
      const name = p.name;
      return tx`Give a value for callers to pass for '${name}'`;
    }
  }
  if (sig.extension && rows[0]?.from !== 0) return tx`The 'this' parameter of an extension method stays first`;
  const params = parsed.findIndex((p) => p.modifiers.includes("params"));
  if (params >= 0 && params !== rows.length - 1) return tx`A params parameter must be last`;
  return null;
}

/** Whether the rows are the signature as it is. */
export function unchanged(sig: MethodSignature, rows: readonly SignatureRow[]): boolean {
  return (
    rows.length === sig.parameters.length &&
    rows.every((r, i) => r.from === i && r.text.trim() === sig.parameters[i]!.text.trim())
  );
}

export function toChange(rows: readonly SignatureRow[]): NewParameter[] {
  return rows.map((r) => ({
    from: r.from,
    text: r.text.trim(),
    value: r.from === null && r.value.trim() ? r.value.trim() : null,
  }));
}

/** The declaration header with the new parameter list, for the dialog's preview. */
export function preview(sig: MethodSignature, rows: readonly SignatureRow[]): string {
  const list = rows.map((r) => r.text.trim()).join(", ");
  const h = sig.header;
  const at = new RegExp(`\\b${sig.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*(<[^()]*>)?\\s*\\(`).exec(h);
  if (!at) return `${sig.name}(${list})`;
  const open = at.index + at[0].length - 1;
  let depth = 0;
  for (let i = open; i < h.length; i++) {
    if (h[i] === "(") depth++;
    else if (h[i] === ")" && --depth === 0) return `${h.slice(0, open + 1)}${list}${h.slice(i)}`;
  }
  return `${h.slice(0, open + 1)}${list})`;
}
