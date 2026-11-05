/**
 * Live text of open `.axaml` files for the Avalonia previewer: an editor
 * contribution publishes the document text, debounced while typing, per path.
 */
import { ViewPlugin, type EditorView, type ViewUpdate } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { create } from "zustand";
import type { EditorDocContext } from "@/editor/api";
import { isVirtualUri } from "@/platform/virtualDocs";

/** Avalonia XAML; WPF and MAUI `.xaml` have no previewer host. */
export const AVALONIA_XAML = /\.axaml$/i;

export const useLiveXaml = create<Record<string, string | undefined>>(() => ({}));

const TEXT_DELAY = 250;

function publish(path: string, text: string) {
  if (useLiveXaml.getState()[path] !== text) useLiveXaml.setState({ [path]: text });
}

export function xamlLiveText(ctx: EditorDocContext): Extension {
  if (!AVALONIA_XAML.test(ctx.path) || isVirtualUri(ctx.path)) return [];
  const path = ctx.path;
  return ViewPlugin.fromClass(
    class {
      timer: ReturnType<typeof setTimeout> | null = null;
      constructor(readonly view: EditorView) {
        publish(path, view.state.doc.toString());
      }
      update(u: ViewUpdate) {
        if (!u.docChanged) return;
        if (this.timer) clearTimeout(this.timer);
        this.timer = setTimeout(() => {
          this.timer = null;
          publish(path, this.view.state.doc.toString());
        }, TEXT_DELAY);
      }
      destroy() {
        if (this.timer) clearTimeout(this.timer);
      }
    },
  );
}
