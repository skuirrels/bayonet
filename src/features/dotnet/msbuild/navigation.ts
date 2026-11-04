/**
 * Go to Declaration in MSBuild files (SLN-5, CI-18): a central package version, a
 * property's definition in the file or in `Directory.Build.props/targets`, the
 * files `Import` and `ProjectReference` point at, and targets named in
 * `DependsOnTargets` / `AfterTargets` / `BeforeTargets` / `Targets`, including
 * the .NET SDK's.
 */
import { EditorState, type Extension } from "@codemirror/state";
import { commands } from "@/bindings";
import { CONTRIBUTED_DEFINITION, type ContributedDefinition, type EditorDocContext } from "@/editor/api";
import { showHint } from "@/editor/lsp/hint";
import { isVirtualUri } from "@/platform/virtualDocs";
import { call } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { editorService } from "@/workbench/contributions";

export const MSBUILD_FILE = /\.(csproj|fsproj|vbproj|sqlproj|esproj|props|targets)$/i;

export type MsbuildTarget =
  | { kind: "package"; id: string }
  | { kind: "property"; name: string }
  | { kind: "path"; path: string }
  | { kind: "target"; name: string };

/** Attributes whose value is a `;`-separated list of target names. */
const TARGET_LIST =
  /\b(?:DependsOnTargets|AfterTargets|BeforeTargets|Targets|InitialTargets|DefaultTargets)\s*=\s*"([^"]*)"/gi;

export interface TextLocation {
  line: number;
  col: number;
}

/** The thing at `offset` that Go to Declaration can follow, if any. */
export function msbuildTargetAt(text: string, offset: number): MsbuildTarget | null {
  const start = text.lastIndexOf("\n", offset - 1) + 1;
  const end = text.indexOf("\n", offset);
  const line = text.slice(start, end < 0 ? text.length : end);
  const at = offset - start;
  const within = (m: RegExpExecArray, group: number) => {
    const from = m.index + m[0].lastIndexOf(m[group]!);
    return at >= from && at <= from + m[group]!.length;
  };
  for (const m of line.matchAll(/\$\(([A-Za-z_][\w.-]*)\)/g))
    if (at >= m.index && at <= m.index + m[0].length) return { kind: "property", name: m[1]! };
  const pkg = /<PackageReference\b[^>]*?\b(?:Include|Update)\s*=\s*"([^"$]+)"/i.exec(line);
  if (pkg && (within(pkg, 1) || (at >= pkg.index && at <= pkg.index + "<PackageReference".length)))
    return { kind: "package", id: pkg[1]!.trim() };
  const file = /<(?:Import|ProjectReference)\b[^>]*?\b(?:Project|Include)\s*=\s*"([^"]+)"/i.exec(line);
  if (file && within(file, 1)) return { kind: "path", path: file[1]! };
  for (const m of line.matchAll(TARGET_LIST)) {
    const from = m.index + m[0].length - 1 - m[1]!.length;
    if (at < from || at > from + m[1]!.length) continue;
    // The name under the caret in `A;B;$(C)`.
    const rel = at - from;
    const start = m[1]!.lastIndexOf(";", rel - 1) + 1;
    const stop = m[1]!.indexOf(";", rel);
    const name = m[1]!.slice(start, stop < 0 ? undefined : stop).trim();
    return /^[A-Za-z_][\w.-]*$/.test(name) ? { kind: "target", name } : null;
  }
  return null;
}

/** Where `<Target Name="name">` is in `text`, outside comments (names ignore case). */
export function findTargetDefinition(text: string, name: string): TextLocation | null {
  const clean = text.replace(/<!--[\s\S]*?-->/g, (c) => c.replace(/[^\n]/g, " "));
  const want = name.toLowerCase();
  for (const m of clean.matchAll(/<Target\s[^>]*?\bName\s*=\s*["']([^"']+)["']/g))
    if (m[1]!.trim().toLowerCase() === want) return locationOf(text, m.index);
  return null;
}

/** The files `<Import Project="…">` in `text` (in `dir`) name, where they can be resolved without evaluation. */
export function importedFiles(text: string, dir: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/<Import\b[^>]*?\bProject\s*=\s*"([^"]+)"/gi)) {
    const file = resolveMsbuildPath(dir, m[1]!);
    if (file) out.push(file);
  }
  return out;
}

/** Where `<PackageVersion Include="id">` is in a `Directory.Packages.props` (ids ignore case, as NuGet's do). */
export function findPackageVersion(text: string, id: string): TextLocation | null {
  const want = id.toLowerCase();
  for (const m of text.matchAll(/<PackageVersion\b[^>]*?\b(?:Include|Update)\s*=\s*"([^"]+)"/gi))
    if (m[1]!.trim().toLowerCase() === want) return locationOf(text, m.index);
  return null;
}

/** The first element that sets property `name` (`<Name>` or `<Name Condition=…>`), outside comments. */
export function findPropertyDefinition(text: string, name: string): TextLocation | null {
  const clean = text.replace(/<!--[\s\S]*?-->/g, (c) => c.replace(/[^\n]/g, " "));
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = new RegExp(`<${escaped}(?=[\\s>/])`, "i").exec(clean);
  return m ? locationOf(text, m.index) : null;
}

export function locationOf(text: string, index: number): TextLocation {
  const before = text.slice(0, index);
  const line = before.split("\n").length - 1;
  return { line, col: index - (before.lastIndexOf("\n") + 1) };
}

const parentOf = (p: string) => p.slice(0, Math.max(0, p.lastIndexOf("/")));

/** `dir` and its ancestors, nearest first. */
export function ancestors(dir: string): string[] {
  const out: string[] = [];
  for (let d = dir; d; d = parentOf(d)) out.push(d);
  return out;
}

/** An `Import` / `ProjectReference` path relative to the file in `dir` (Windows separators allowed). */
export function resolveMsbuildPath(dir: string, value: string): string | null {
  const v = value
    .trim()
    .replace(/\$\(MSBuildThisFileDirectory\)/gi, `${dir}/`)
    .replace(/\$\((?:MSBuildProjectDirectory|MSBuildThisFileDirectoryNoRoot)\)/gi, dir)
    .replace(/\\/g, "/");
  // Other properties and wildcards depend on evaluation.
  if (!v || /\$\(|[*?]/.test(v)) return null;
  const full = v.startsWith("/") || /^[A-Za-z]:\//.test(v) ? v : `${dir}/${v}`;
  const parts: string[] = [];
  for (const seg of full.split("/")) {
    if (!seg || seg === ".") continue;
    // Never above the root (or a Windows drive).
    if (seg === "..") {
      if (parts.length > 1 || (parts.length === 1 && !/^[A-Za-z]:$/.test(parts[0]!))) parts.pop();
    } else parts.push(seg);
  }
  return full.startsWith("/") ? `/${parts.join("/")}` : parts.join("/");
}

async function readText(path: string): Promise<string | null> {
  try {
    return await call(commands.fsReadText(path));
  } catch {
    return null;
  }
}

/** The nearest `name` file in `dir` or above, with its text. */
async function nearest(dir: string, name: string, skip?: string): Promise<Array<{ path: string; text: string }>> {
  const out: Array<{ path: string; text: string }> = [];
  for (const d of ancestors(dir)) {
    const path = `${d}/${name}`;
    if (path === skip) continue;
    const text = await readText(path);
    if (text !== null) out.push({ path, text });
  }
  return out;
}

const open = (path: string, at: TextLocation) =>
  editorService().openFile(path, { selection: { line: at.line, col: at.col }, focus: true });

/** Opens what `target` (in the MSBuild file `path`, whose text is `text`) refers to; `false` when nothing is found. */
export async function followMsbuildTarget(path: string, text: string, target: MsbuildTarget): Promise<boolean> {
  const dir = parentOf(path);
  switch (target.kind) {
    case "package": {
      // MSBuild imports only the nearest Directory.Packages.props; a nested one may import its parent.
      for (const f of await nearest(dir, "Directory.Packages.props")) {
        const at = findPackageVersion(f.text, target.id);
        if (at) {
          await open(f.path, at);
          return true;
        }
      }
      return false;
    }
    case "property": {
      const own = findPropertyDefinition(text, target.name);
      if (own) {
        await open(path, own);
        return true;
      }
      for (const name of ["Directory.Build.props", "Directory.Build.targets"])
        for (const f of await nearest(dir, name, path)) {
          const at = findPropertyDefinition(f.text, target.name);
          if (at) {
            await open(f.path, at);
            return true;
          }
        }
      return false;
    }
    case "path": {
      const file = resolveMsbuildPath(dir, target.path);
      if (!file || (await readText(file)) === null) return false;
      await open(file, { line: 0, col: 0 });
      return true;
    }
    case "target": {
      const own = findTargetDefinition(text, target.name);
      if (own) {
        await open(path, own);
        return true;
      }
      const files: Array<{ path: string; text: string }> = [];
      for (const name of ["Directory.Build.props", "Directory.Build.targets"])
        files.push(...(await nearest(dir, name, path)));
      for (const file of importedFiles(text, dir)) {
        const t = await readText(file);
        if (t !== null) files.push({ path: file, text: t });
      }
      for (const f of files) {
        const at = findTargetDefinition(f.text, target.name);
        if (at) {
          await open(f.path, at);
          return true;
        }
      }
      const sdk = await call(commands.dotnetSdkTarget(target.name)).catch(() => null);
      if (!sdk) return false;
      await open(sdk.path, { line: sdk.line, col: sdk.col });
      return true;
    }
  }
}

/** Editor contribution: MSBuild Go to Declaration ahead of the language server. */
export function msbuildNavigation(ctx: EditorDocContext): Extension {
  if (isVirtualUri(ctx.path) || !MSBUILD_FILE.test(ctx.path)) return [];
  const definition: ContributedDefinition = (view, pos) => {
    const text = view.state.doc.toString();
    const target = msbuildTargetAt(text, pos);
    if (!target) return false;
    void followMsbuildTarget(ctx.path, text, target).then((found) => {
      if (!found && view.dom.isConnected) showHint(view, tx`Cannot find declaration to go to`, { pos });
    });
    return true;
  };
  return EditorState.languageData.of(() => [{ [CONTRIBUTED_DEFINITION]: definition }]);
}
