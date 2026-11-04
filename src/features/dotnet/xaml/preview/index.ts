/** Avalonia previewer (CI-16): the split editor of `.axaml` files and its layout actions. */
import { registerActions, type Action } from "@/platform/actions";
import { getContext } from "@/platform/context";
import { lazyComponent } from "@/ui";
import { registerTextEditorWrapper } from "@/workbench/contributions";
import { AVALONIA_XAML } from "./live";
import { setXamlMode, type XamlMode } from "./modes";

export { xamlLiveText } from "./live";

function layoutActions(): Action[] {
  const mode = (id: string, title: string, m: XamlMode, icon: string, description: string): Action => ({
    id,
    title,
    description,
    category: "Avalonia",
    icon,
    keywords: ["avalonia", "xaml", "axaml", "preview", "previewer", "designer", "layout"],
    when: "textEditorActive",
    run: ({ arg }) => {
      const path = typeof arg === "string" ? arg : getContext().editorPath;
      if (typeof path === "string" && AVALONIA_XAML.test(path)) setXamlMode(path, m);
    },
  });
  return [
    mode("dotnet.xaml.editorOnly", "Editor Only", "editor", "editor-only", "Shows only the XAML text"),
    mode(
      "dotnet.xaml.editorAndPreview",
      "Editor and Preview",
      "split",
      "editor-preview",
      "Shows the XAML text beside the Avalonia preview",
    ),
    mode("dotnet.xaml.previewOnly", "Preview Only", "preview", "preview-only", "Shows only the Avalonia preview"),
  ];
}

export function activateXamlPreview(): () => void {
  const disposers = [
    registerTextEditorWrapper({
      id: "dotnet.avaloniaPreview",
      matches: (path) => AVALONIA_XAML.test(path),
      component: lazyComponent(() => import("./XamlPreviewEditor").then((m) => m.XamlPreviewEditor)),
      tabActions: lazyComponent(() => import("./XamlPreviewEditor").then((m) => m.XamlLayoutActions)),
    }),
    registerActions(layoutActions()),
  ];
  return () => disposers.reverse().forEach((d) => d());
}
