/**
 * XAML element, attribute and value completion (CI-16) as text: what is being typed at
 * a position, and what the UI framework and project types (read from the project's
 * assemblies) offer there. A prefix resolves through the file's `xmlns` declarations:
 * `clr-namespace:` / `using:` name a CLR namespace, any other value is an XML namespace
 * the framework maps with `XmlnsDefinitionAttribute`.
 */
import type { XamlProperty, XamlType, XamlTypes } from "@/bindings";
import { enclosingElements } from "./model";

export interface XamlCatalog {
  byFullName: Map<string, XamlType>;
  /** XML namespace to the CLR namespaces mapped to it. */
  byXmlns: Map<string, string[]>;
  byNamespace: Map<string, XamlType[]>;
}

export function catalog(types: XamlTypes): XamlCatalog {
  const byFullName = new Map<string, XamlType>();
  const byNamespace = new Map<string, XamlType[]>();
  for (const t of types.types) {
    const full = `${t.namespace}.${t.name}`;
    if (byFullName.has(full)) continue;
    byFullName.set(full, t);
    const list = byNamespace.get(t.namespace) ?? [];
    list.push(t);
    byNamespace.set(t.namespace, list);
  }
  const byXmlns = new Map<string, string[]>();
  for (const m of types.namespaces) {
    const list = byXmlns.get(m.xmlns) ?? [];
    if (!list.includes(m.clrNamespace)) list.push(m.clrNamespace);
    byXmlns.set(m.xmlns, list);
  }
  return { byFullName, byXmlns, byNamespace };
}

/** `xmlns` prefixes of the file (`""` for the default) to the CLR namespaces they bring in. */
export function prefixes(text: string, cat: XamlCatalog): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const m of text.matchAll(/\bxmlns(?::([\w.-]+))?\s*=\s*"([^"]*)"/g)) {
    const value = m[2]!.trim();
    const clr = /^(?:clr-namespace|using):([^;]+)/.exec(value)?.[1]?.trim();
    out.set(m[1] ?? "", clr ? [clr] : (cat.byXmlns.get(value) ?? []));
  }
  return out;
}

/** The type an element name (`Button`, `local:Card`) stands for. */
export function resolveElement(name: string, scope: Map<string, string[]>, cat: XamlCatalog): XamlType | null {
  const colon = name.indexOf(":");
  const [prefix, bare] = colon >= 0 ? [name.slice(0, colon), name.slice(colon + 1)] : ["", name];
  for (const ns of scope.get(prefix) ?? []) {
    const t = cat.byFullName.get(`${ns}.${bare}`);
    if (t) return t;
  }
  return null;
}

export interface Members {
  properties: XamlProperty[];
  events: string[];
}

/** A type's properties and events with the inherited ones, nearest first. */
export function membersOf(type: XamlType, cat: XamlCatalog): Members {
  const properties = new Map<string, XamlProperty>();
  const events = new Set<string>();
  const seen = new Set<string>();
  for (let t: XamlType | undefined = type; t; t = t.base ? cat.byFullName.get(t.base) : undefined) {
    const full = `${t.namespace}.${t.name}`;
    if (seen.has(full)) break;
    seen.add(full);
    for (const p of t.properties) if (!properties.has(p.name)) properties.set(p.name, p);
    for (const e of t.events) events.add(e);
  }
  return { properties: [...properties.values()], events: [...events] };
}

/** Every type the file can name, with the name it is written as (`Button`, `local:Card`). */
export function typesInScope(scope: Map<string, string[]>, cat: XamlCatalog): { written: string; type: XamlType }[] {
  const out: { written: string; type: XamlType }[] = [];
  const seen = new Set<string>();
  for (const [prefix, namespaces] of scope) {
    for (const ns of namespaces) {
      for (const type of cat.byNamespace.get(ns) ?? []) {
        const written = prefix ? `${prefix}:${type.name}` : type.name;
        if (seen.has(written)) continue;
        seen.add(written);
        out.push({ written, type });
      }
    }
  }
  return out;
}

export type XamlSite =
  /** After `<`: an element, or a property element of the enclosing one (`Grid.RowDefinitions`). */
  | { kind: "element"; from: number; parent: string | null }
  /** An attribute name in the start tag of `element`. */
  | { kind: "attribute"; from: number; element: string }
  /** The value of `attribute` of `element`, outside a markup extension. */
  | { kind: "value"; from: number; element: string; attribute: string };

/** What is being typed at `pos`, if XAML completion has something for it. */
export function xamlSiteAt(text: string, pos: number): XamlSite | null {
  const before = text.slice(0, pos);
  if (before.lastIndexOf("<!--") > before.lastIndexOf("-->")) return null;
  const open = before.lastIndexOf("<");
  if (open < 0 || before.lastIndexOf(">") > open) return null;
  const tag = before.slice(open);
  if (/^<[!?/]/.test(tag)) return null;
  if (/^<[\w.:-]*$/.test(tag)) {
    const parent = enclosingElements(text, open).at(-1)?.name ?? null;
    return { kind: "element", from: open + 1, parent };
  }
  const element = /^<([\w.:-]+)/.exec(tag)?.[1];
  if (!element) return null;
  if ((tag.match(/"/g)?.length ?? 0) % 2 === 1) {
    const m = /\s([\w.:]+)\s*=\s*"([^"]*)$/.exec(tag);
    if (!m || m[2]!.trimStart().startsWith("{")) return null;
    return { kind: "value", from: pos - m[2]!.length, element, attribute: m[1]! };
  }
  const name = /\s([\w.:]*)$/.exec(tag);
  return name ? { kind: "attribute", from: pos - name[1]!.length, element } : null;
}

export interface XamlOption {
  label: string;
  kind: "type" | "property" | "event" | "enum";
  detail?: string;
}

const shortType = (t: string) => t.replace(/^System\./, "").replace(/(?:[\w]+\.)+(\w+)/g, "$1");

/** What completes at `site`. */
export function xamlOptions(text: string, site: XamlSite, cat: XamlCatalog): XamlOption[] {
  const scope = prefixes(text, cat);
  if (site.kind === "element") {
    const out: XamlOption[] = typesInScope(scope, cat)
      .filter(({ type }) => !type.abstract)
      .map(({ written, type }) => ({ label: written, kind: "type", detail: type.namespace }));
    const parent = site.parent && !site.parent.includes(".") ? resolveElement(site.parent, scope, cat) : null;
    if (parent) {
      for (const p of membersOf(parent, cat).properties) {
        out.push({ label: `${site.parent}.${p.name}`, kind: "property", detail: shortType(p.type) });
      }
    }
    return out;
  }
  const type = resolveElement(site.element, scope, cat);
  if (site.kind === "attribute") {
    const out: XamlOption[] = [];
    if (type) {
      const { properties, events } = membersOf(type, cat);
      for (const p of properties) out.push({ label: p.name, kind: "property", detail: shortType(p.type) });
      for (const e of events) out.push({ label: e, kind: "event" });
    }
    for (const { written, type: owner } of typesInScope(scope, cat)) {
      for (const p of owner.attached)
        out.push({ label: `${written}.${p.name}`, kind: "property", detail: shortType(p.type) });
    }
    return out;
  }
  const values = valuesOf(site.attribute, type, scope, cat);
  return values.map((v) => ({ label: v, kind: "enum" }));
}

function valuesOf(attribute: string, type: XamlType | null, scope: Map<string, string[]>, cat: XamlCatalog): string[] {
  const dot = attribute.lastIndexOf(".");
  if (dot > 0) {
    const owner = resolveElement(attribute.slice(0, dot), scope, cat);
    const name = attribute.slice(dot + 1);
    return owner?.attached.find((p) => p.name === name)?.values ?? [];
  }
  if (!type) return [];
  return membersOf(type, cat).properties.find((p) => p.name === attribute)?.values ?? [];
}
