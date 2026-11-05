/**
 * XAML (CI-16) as text: the CLR namespaces behind `xmlns` prefixes, the type
 * bindings resolve against at a position (`x:DataType`, `d:DataContext`, a
 * `DataTemplate`'s `DataType`, or the `x:Class` for `x:Bind`), and the binding
 * path being typed. WPF, Avalonia, MAUI and WinUI files share these shapes.
 */

export const XAML_FILE = /\.(xaml|axaml)$/i;

export interface TypeRef {
  /** `null` when the XAML does not say (an unprefixed name). */
  namespace: string | null;
  name: string;
}

/** `xmlns` prefixes (`""` for the default namespace) mapped to CLR namespaces. */
export function clrNamespaces(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const m of text.matchAll(/\bxmlns(?::([\w.-]+))?\s*=\s*"([^"]*)"/g)) {
    const value = m[2]!.trim();
    const ns = /^clr-namespace:([^;]+)/.exec(value)?.[1] ?? /^using:([^;]+)/.exec(value)?.[1];
    if (ns) out.set(m[1] ?? "", ns.trim());
  }
  return out;
}

/** `vm:MainViewModel`, `{x:Type vm:MainViewModel}` or `App.ViewModels.MainViewModel` as a type. */
export function typeRef(written: string, namespaces: Map<string, string>): TypeRef | null {
  let t = written.trim();
  const ext = /^\{\s*(?:x:Type\s+)?(?:Type\s*=\s*)?([^,}\s]+)/.exec(t);
  if (ext) t = ext[1]!;
  const colon = t.indexOf(":");
  if (colon >= 0) {
    const ns = namespaces.get(t.slice(0, colon));
    const name = t.slice(colon + 1);
    return /^[\w.]+$/.test(name) ? { namespace: ns ?? null, name } : null;
  }
  if (!/^[\w.]+$/.test(t)) return null;
  const dot = t.lastIndexOf(".");
  if (dot > 0) return { namespace: t.slice(0, dot), name: t.slice(dot + 1) };
  return { namespace: namespaces.get("") ?? null, name: t };
}

interface Element {
  name: string;
  /** The start tag, `<` to `>`. */
  tag: string;
  start: number;
}

/** Start tags enclosing `pos`, outermost first; the tag `pos` is in counts. */
export function enclosingElements(text: string, pos: number): Element[] {
  const stack: Element[] = [];
  const tags =
    /<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<\?[\s\S]*?\?>|<(\/?)([\w.:-]+)((?:[^>"']|"[^"]*"|'[^']*')*)>/g;
  for (const m of text.matchAll(tags)) {
    const start = m.index;
    if (start >= pos) break;
    if (!m[2]) continue;
    const end = start + m[0].length;
    const tag: Element = { name: m[2], tag: m[0], start };
    if (m[1]) {
      const at = stack.map((e) => e.name).lastIndexOf(m[2]);
      if (at >= 0 && end <= pos) stack.length = at;
    } else if (m[3]!.endsWith("/")) {
      if (end > pos) stack.push(tag);
    } else stack.push(tag);
  }
  return stack;
}

/** The value of `attribute` in a start tag. */
export function attribute(tag: string, name: string): string | null {
  const re = new RegExp(`\\s${name.replace(/[.:]/g, "\\$&")}\\s*=\\s*"([^"]*)"`);
  return re.exec(tag)?.[1] ?? null;
}

/** What `{Binding}` resolves against at `pos` (`xBind`: the code-behind class instead). */
export function dataTypeAt(text: string, pos: number, xBind = false): TypeRef | null {
  const namespaces = clrNamespaces(text);
  const elements = enclosingElements(text, pos);
  if (xBind) {
    const cls = elements.length ? attribute(elements[0]!.tag, "x:Class") : null;
    return cls ? typeRef(cls, namespaces) : null;
  }
  for (const e of [...elements].reverse()) {
    const dataType = attribute(e.tag, "x:DataType");
    if (dataType) return typeRef(dataType, namespaces);
    const design = attribute(e.tag, "d:DataContext");
    const instance = design && /DesignInstance\s+(?:Type\s*=\s*)?([\w.:]+)/.exec(design);
    if (instance) return typeRef(instance[1]!, namespaces);
    if (/(^|[:.])DataTemplate$/.test(e.name)) {
      const templated = attribute(e.tag, "DataType");
      if (templated) return typeRef(templated, namespaces);
    }
  }
  return null;
}

export interface BindingPath {
  /** The path up to `pos`, e.g. `Owner.Address.Ci`. */
  path: string;
  /** Where the segment being typed starts. */
  from: number;
  xBind: boolean;
}

const BINDING = /\{\s*(Binding|x:Bind|CompiledBinding|ReflectionBinding)(?=[\s}]|$)([^{}]*)$/;

/** The binding path `pos` is in (or right after), if any. */
export function bindingPathAt(text: string, pos: number): BindingPath | null {
  const before = text.slice(0, pos);
  const open = before.lastIndexOf("<");
  if (open < 0 || before.lastIndexOf(">") > open) return null;
  const tag = before.slice(open);
  const quotes = tag.match(/"/g)?.length ?? 0;
  if (quotes % 2 === 0) return null;
  const value = tag.slice(tag.lastIndexOf('"') + 1);
  const m = BINDING.exec(value);
  if (!m) return null;
  const args = m[2]!;
  const parts = args.split(",");
  const last = parts.at(-1)!.trimStart();
  let path: string;
  const named = /^(\w+)\s*=\s*(.*)$/.exec(last);
  if (named) {
    if (named[1] !== "Path") return null;
    path = named[2]!;
  } else {
    if (parts.length > 1) return null;
    path = last;
  }
  if (!/^[\w.[\]()]*$/.test(path)) return null;
  const current = /[\w]*$/.exec(path)![0];
  return { path, from: pos - current.length, xBind: m[1] === "x:Bind" };
}

/** Segments before the one being typed: `Items[0].Owner.Na` is `Items[0]`, `Owner`. */
export function leadingSegments(path: string): string[] {
  const parts = path.split(".");
  parts.pop();
  return parts.filter(Boolean);
}

/** The element type of a collection type: `ObservableCollection<Item>?` is `Item`. */
export function elementType(typeName: string): string {
  const t = typeName.trim().replace(/\?$/, "");
  if (t.endsWith("[]")) return t.slice(0, -2);
  const args = /<(.+)>$/.exec(t)?.[1];
  if (!args) return t;
  // The last generic argument: `Dictionary<string, Item>` holds `Item`s.
  let depth = 0;
  let cut = 0;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "<") depth++;
    else if (args[i] === ">") depth--;
    else if (args[i] === "," && depth === 0) cut = i + 1;
  }
  return args.slice(cut).trim();
}

/** A type as a binding path continues into it: `Models.User?` is `User` (in `Models`). */
export function namedType(typeName: string): TypeRef {
  const t = typeName.trim().replace(/\?$/, "").replace(/<.*$/, "");
  const dot = t.lastIndexOf(".");
  return dot > 0 ? { namespace: t.slice(0, dot), name: t.slice(dot + 1) } : { namespace: null, name: t };
}

/** The type named at `pos`: an element tag (`<local:Card`) or a type-valued attribute. */
export function typeNameAt(text: string, pos: number): TypeRef | null {
  const namespaces = clrNamespaces(text);
  const word = (s: number, e: number) => {
    let a = pos;
    let b = pos;
    while (a > s && /[\w.:]/.test(text[a - 1]!)) a--;
    while (b < e && /[\w.]/.test(text[b]!)) b++;
    return text.slice(a, b);
  };
  const before = text.slice(0, pos);
  const open = before.lastIndexOf("<");
  if (open < 0 || before.lastIndexOf(">") > open) return null;
  const head = /^<\/?([\w.:-]*)/.exec(text.slice(open))!;
  if (pos <= open + head[0].length) {
    const name = head[1]!;
    return name.includes(":") ? typeRef(name, namespaces) : null;
  }
  const tag = text.slice(open, pos);
  if ((tag.match(/"/g)?.length ?? 0) % 2 === 0) return null;
  const attr = /([\w.:]+)\s*=\s*"[^"]*$/.exec(tag)?.[1];
  if (!attr || !/^(x:DataType|DataType|x:Class|TargetType|d:DataContext|x:TypeArguments)$/.test(attr)) return null;
  const valueEnd = text.indexOf('"', pos);
  const w = word(open, valueEnd < 0 ? text.length : valueEnd);
  return w && !/^(x:Type|d:DesignInstance|Type)$/.test(w) ? typeRef(w, namespaces) : null;
}
