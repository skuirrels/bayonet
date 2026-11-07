/**
 * Change Signature for C# (CI-6), run when no language server offers it:
 * the signature at the caret (the callee when the caret is on a call), the
 * dialog, the server's references, then the backend's rewrite of the
 * declaration and every call, applied as one workspace edit.
 */
import { Channel } from "@tauri-apps/api/core";
import type { EditorView } from "@codemirror/view";
import type { Location, SkippedSite } from "@/bindings";
import { whenSynced } from "@/editor/api";
import { docOf, lspRequest } from "@/editor/lsp/lsp";
import { offsetToPos } from "@/editor/lsp/positions";
import { showHint } from "@/editor/lsp/hint";
import { withDocument } from "@/features/code-insight/ipc";
import { tx } from "@/platform/i18n";
import { call, commands, isTauri, nextRequestId } from "@/platform/ipc";
import { basename } from "@/platform/project";
import { notifications } from "@/workbench/contributions";
import { askSignatureChange } from "./ChangeSignatureDialog";

const notify = (level: "info" | "warning" | "error", title: string, body?: string) =>
  notifications().notify({ group: "refactoring", level, title, body });

/** Lines naming the call sites left alone, at most five. */
export function skippedSummary(skipped: readonly SkippedSite[]): string {
  const lines = skipped.slice(0, 5).map((s) => `${basename(s.path)}:${s.range.start.line + 1}: ${s.reason}`);
  const more = skipped.length - lines.length;
  if (more > 0) lines.push(tx`and ${more} more`);
  return lines.join("\n");
}

/** The fallback for `refactor.changeSignature` in C# files; always handles the request. */
export async function changeCSharpSignature(view: EditorView): Promise<boolean> {
  const ctx = docOf(view);
  if (!ctx || !isTauri()) return false;
  await whenSynced(view);
  const caret = offsetToPos(view.state.doc, view.state.selection.main.head);
  let target = { path: ctx.path, pos: caret };
  try {
    const defs = await lspRequest(() => commands.lspGoto(ctx.doc, caret, "definition"));
    const def = defs.find((d) => !d.uri && /\.cs$/i.test(d.path));
    if (def) target = { path: def.path, pos: def.range.start };
  } catch {
    // No server yet: the declaration around the caret.
  }
  const sig = await call(commands.signatureAt(target.path, target.pos)).catch(() => null);
  if (!sig) {
    showHint(
      view,
      tx`Cannot perform refactoring. Caret should be on a method, constructor, local function or delegate`,
    );
    return true;
  }
  const change = await askSignatureChange(sig);
  if (!change) return true;
  const name = sig.name;
  try {
    const refs = await withDocument(target.path, (doc) =>
      lspRequest(() =>
        commands.lspReferences(nextRequestId(), doc, sig.nameRange.start, true, true, false, new Channel()),
      ),
    );
    const locations: Location[] = [{ path: target.path, range: sig.nameRange }, ...refs.locations];
    const rewrite = await call(commands.changeSignature(sig, change, locations));
    await call(commands.applyWorkspaceEdit(rewrite.edit));
    const calls = rewrite.calls;
    if (rewrite.skipped.length) {
      const count = rewrite.skipped.length;
      notify("warning", tx`Changed '${name}': ${count} usages need a manual update`, skippedSummary(rewrite.skipped));
    } else {
      notify("info", tx`Changed the signature of '${name}' and ${calls} calls`);
    }
  } catch (e) {
    notify("error", tx`Cannot change the signature of '${name}'`, e instanceof Error ? e.message : String(e));
  }
  return true;
}
