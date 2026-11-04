/**
 * Text editor wrapper of Avalonia `.axaml` files (CI-16): the editor, the
 * live preview from Avalonia's designer host, or both side by side.
 */
import { useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { FloatingToolbar, Icon, IconButton } from "@/ui";
import { tx } from "@/platform/i18n";
import { editorService } from "@/workbench/contributions";
import { useLiveXaml } from "./live";
import { modeOf, useXamlModes } from "./modes";
import { useXamlPreview, type PreviewState } from "./session";
import styles from "./XamlPreview.module.css";

export function XamlPreviewEditor({
  path,
  tabId,
  children,
  actionsInTabs = false,
}: {
  path: string;
  tabId: string;
  children: ReactNode;
  actionsInTabs?: boolean;
}) {
  const mode = useXamlModes((s) => modeOf(s, path));
  const ref = useRef<HTMLDivElement>(null);
  return (
    <div ref={ref} className={clsx(styles.split, styles[`mode-${mode}`])}>
      <div className={styles.editor}>{children}</div>
      {mode !== "editor" && <XamlPreview path={path} />}
      {!actionsInTabs && (
        <FloatingToolbar targetRef={ref}>
          <XamlLayoutActions path={path} tabId={tabId} />
        </FloatingToolbar>
      )}
    </div>
  );
}

export function XamlLayoutActions({ path }: { path: string; tabId: string }) {
  const mode = useXamlModes((s) => modeOf(s, path));
  return (
    <div className={styles.layoutActions} role="toolbar" aria-label={tx("XAML editor layout")}>
      <IconButton
        icon="editor-only"
        iconSize={16}
        size="small"
        title="Editor Only"
        action="dotnet.xaml.editorOnly"
        actionArg={path}
        pressed={mode === "editor"}
      />
      <IconButton
        icon="editor-preview"
        iconSize={16}
        size="small"
        title="Editor and Preview"
        action="dotnet.xaml.editorAndPreview"
        actionArg={path}
        pressed={mode === "split"}
      />
      <IconButton
        icon="preview-only"
        iconSize={16}
        size="small"
        title="Preview Only"
        action="dotnet.xaml.previewOnly"
        actionArg={path}
        pressed={mode === "preview"}
      />
    </div>
  );
}

const ZOOMS = [0.25, 0.5, 0.75, 1, 1.5, 2, 3];

function statusText(state: PreviewState): string | null {
  switch (state.phase) {
    case "idle":
    case "building":
      return tx("Building the project…");
    case "starting":
      return tx("Starting the Avalonia previewer…");
    case "failed":
      return state.message ?? tx("The preview failed");
    case "stopped":
      return tx("The previewer stopped");
    case "ready":
      return state.frame ? null : tx("Rendering…");
  }
}

function XamlPreview({ path }: { path: string }) {
  const text = useLiveXaml((s) => s[path]);
  const [generation, setGeneration] = useState(0);
  const [zoom, setZoom] = useState(1);
  const state = useXamlPreview(path, text, generation);
  const status = statusText(state);
  const failed = state.phase === "failed";
  const frame = state.frame;
  const step = (dir: 1 | -1) => {
    const i = ZOOMS.indexOf(zoom);
    const next = ZOOMS[Math.min(ZOOMS.length - 1, Math.max(0, (i < 0 ? 3 : i) + dir))];
    if (next != null) setZoom(next);
  };
  const goToError = () => {
    const e = state.error;
    if (e?.line == null) return;
    void editorService().openFile(path, { selection: { line: e.line, col: e.column ?? 0 }, focus: true });
  };
  return (
    <section className={styles.preview} aria-label={tx("Avalonia preview")}>
      <div className={styles.toolbar}>
        <span className={clsx(styles.status, failed && styles.failed)} role="status">
          {status ?? tx`${Math.round(zoom * 100)}%`}
        </span>
        <span className={styles.spacer} />
        <IconButton icon="zoom-out" size="small" title="Zoom Out" onClick={() => step(-1)} disabled={!frame} />
        <IconButton icon="actual-size" size="small" title="Actual Size" onClick={() => setZoom(1)} disabled={!frame} />
        <IconButton icon="zoom-in" size="small" title="Zoom In" onClick={() => step(1)} disabled={!frame} />
        <IconButton
          icon="refresh"
          size="small"
          title="Build and Restart the Preview"
          onClick={() => setGeneration((g) => g + 1)}
        />
      </div>
      {state.error && (
        <div className={styles.error} role="alert">
          <Icon name="error" size={14} />
          <span className={styles.errorText}>{state.error.message}</span>
          {state.error.line != null && (
            <button type="button" className={styles.link} onClick={goToError}>
              {tx`Line ${state.error.line + 1}`}
            </button>
          )}
        </div>
      )}
      <div className={styles.canvas}>
        {frame && (
          <img
            className={clsx(styles.frame, state.error && styles.stale)}
            src={frame.src}
            alt={tx("Preview of the XAML")}
            width={(frame.width / frame.scale) * zoom}
            height={(frame.height / frame.scale) * zoom}
            draggable={false}
          />
        )}
      </div>
    </section>
  );
}
