/**
 * XAML completion and navigation (CI-16): binding paths complete with the
 * members of the binding's data type (followed through each segment, into a
 * collection's element type after an indexer), and Go to Declaration opens the
 * C# member or type a binding, element or type attribute names.
 */
import type { Completion, CompletionContext, CompletionSource } from "@codemirror/autocomplete";
import { EditorState, type Extension } from "@codemirror/state";
import { commands, type BindableMember, type BindableType } from "@/bindings";
import { CONTRIBUTED_DEFINITION, type ContributedDefinition, type EditorDocContext } from "@/editor/api";
import { showHint } from "@/editor/lsp/hint";
import { tx } from "@/platform/i18n";
import { call, isTauri } from "@/platform/ipc";
import { isVirtualUri } from "@/platform/virtualDocs";
import { editorService } from "@/workbench/contributions";
import {
  XAML_FILE,
  bindingPathAt,
  dataTypeAt,
  elementType,
  leadingSegments,
  namedType,
  typeNameAt,
  type TypeRef,
} from "./model";

const CACHE_MS = 5_000;
const cache = new Map<string, { at: number; value: Promise<BindableType | null> }>();

/** The project's type `ref` with its members; cached briefly while typing. */
export function typeMembers(ref: TypeRef): Promise<BindableType | null> {
  const key = `${ref.namespace ?? ""}\n${ref.name}`;
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < CACHE_MS) return hit.value;
  const value = call(commands.csharpTypeMembers(ref.name, ref.namespace)).catch(() => null);
  cache.set(key, { at: Date.now(), value });
  return value;
}

/** The type a binding path reaches after `segments`, from `root`. */
export async function resolvePath(root: TypeRef, segments: readonly string[]): Promise<BindableType | null> {
  let type = await typeMembers(root);
  for (const segment of segments) {
    if (!type) return null;
    const name = segment.replace(/[[(].*$/, "");
    const member = type.members.find((m) => m.name === name);
    if (!member?.typeName) return null;
    const next = segment.includes("[") ? elementType(member.typeName) : member.typeName;
    type = await typeMembers(namedType(next));
  }
  return type;
}

function option(m: BindableMember): Completion {
  const owner = m.declaredIn;
  return {
    label: m.name,
    type: m.kind === "command" ? "method" : "property",
    detail: m.typeName ?? "ICommand",
    info: tx`Declared in ${owner}`,
  };
}

const source: CompletionSource = async (ctx: CompletionContext) => {
  const text = ctx.state.doc.toString();
  const site = bindingPathAt(text, ctx.pos);
  if (!site) return null;
  if (!ctx.explicit && site.from === ctx.pos && !/[{\s.=,]$/.test(text.slice(0, ctx.pos))) return null;
  const root = dataTypeAt(text, ctx.pos, site.xBind);
  if (!root) return null;
  const type = await resolvePath(root, leadingSegments(site.path));
  if (!type || ctx.aborted) return null;
  return { from: site.from, options: type.members.map(option), validFor: /^\w*$/ };
};

const open = (path: string, line: number, col: number) =>
  editorService().openFile(path, { selection: { line, col }, focus: true });

/** Opens the member or type named at `pos`; `false` when there is none to go to. */
export async function followXaml(text: string, pos: number): Promise<boolean> {
  // The whole segment under the caret.
  let end = pos;
  while (end < text.length && /\w/.test(text[end]!)) end++;
  const site = bindingPathAt(text, end);
  if (site) {
    const root = dataTypeAt(text, end, site.xBind);
    const word = site.path.slice(site.path.length - (end - site.from));
    const type = root && (await resolvePath(root, leadingSegments(site.path)));
    const member = type?.members.find((m) => m.name === word);
    if (member) {
      await open(member.path, member.range.start.line, member.range.start.col);
      return true;
    }
    return false;
  }
  const named = typeNameAt(text, pos);
  const type = named && (await typeMembers(named));
  if (!type) return false;
  await open(type.path, type.range.start.line, type.range.start.col);
  return true;
}

/** Whether `pos` is on something XAML navigation handles. */
function handles(text: string, pos: number): boolean {
  let end = pos;
  while (end < text.length && /\w/.test(text[end]!)) end++;
  return !!bindingPathAt(text, end) || !!typeNameAt(text, pos);
}

/** Editor contribution: binding completion and navigation in `.xaml` / `.axaml` files. */
export function xamlEditor(ctx: EditorDocContext): Extension {
  if (isVirtualUri(ctx.path) || !XAML_FILE.test(ctx.path) || !isTauri()) return [];
  const definition: ContributedDefinition = (view, pos) => {
    const text = view.state.doc.toString();
    if (!handles(text, pos)) return false;
    void followXaml(text, pos).then((found) => {
      if (!found && view.dom.isConnected) showHint(view, tx`Cannot find declaration to go to`, { pos });
    });
    return true;
  };
  return EditorState.languageData.of(() => [{ openideaCompletion: source, [CONTRIBUTED_DEFINITION]: definition }]);
}
