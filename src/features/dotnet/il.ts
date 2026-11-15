/**
 * IL view (CI-10): "Show IL" opens the IL of the C# type at the caret in a read-only
 * tab. In library code Roslyn decompiled for Go to Definition it comes from the
 * implementation assembly, in the project's own code from its built output. The
 * backend names the document (`dotnet-il:`) and the .NET helper disassembles it.
 */
import { commands } from "@/bindings";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import { virtualDocument } from "@/platform/virtualDocs";
import { basename } from "@/platform/project";
import { maybeEditorService, notifications, registerEditorBanner } from "@/workbench/contributions";
import { upsertTask } from "@/workbench/tasks";

/** A file Roslyn decompiled from a referenced assembly; its header names the assembly. */
export function isDecompiledSource(path: string): boolean {
  return /[\\/]MetadataAsSource[\\/](?:.*[\\/])?DecompilationMetadataAsSourceFileProvider[\\/].*\.cs$/i.test(path);
}

let runs = 0;

/**
 * Opens the IL of the type at the caret of the focused editor, or of the first type
 * of `path` when another file has the caret.
 */
export async function showIl(path?: string): Promise<void> {
  const editor = maybeEditorService();
  if (!editor) return;
  const cursor = editor.activeCursor();
  const file = path ?? cursor?.path;
  if (!file) return;
  const pos = cursor && cursor.path === file ? { line: cursor.line, col: cursor.col } : null;
  // The first use builds the .NET helper, which takes a while.
  const task = { id: `dotnet.il:${++runs}`, title: tx("Disassembling to IL") };
  upsertTask({ ...task, message: basename(file), fraction: null, done: false });
  try {
    const uri = await call(commands.dotnetIlUri(file, pos));
    // Fetched here first so a failure is reported as what it is; opening reuses it.
    await virtualDocument(uri);
    await editor.openFile(uri, { focus: true });
  } catch (e) {
    notifications().notify({
      group: "dotnet",
      level: "warning",
      title: tx`Cannot show IL`,
      body: e instanceof Error ? e.message : String(e),
    });
  } finally {
    upsertTask({ ...task, message: null, fraction: 1, done: true });
  }
}

/** The notice above decompiled library code, with the way to its IL. */
export function registerIlBanner(): () => void {
  return registerEditorBanner({
    id: "dotnet.il",
    order: 20,
    useBanner: (path) =>
      isDecompiledSource(path)
        ? {
            text: "Decompiled from a referenced assembly.",
            actions: [{ label: "Show IL", run: () => void showIl(path) }],
          }
        : null,
  });
}
