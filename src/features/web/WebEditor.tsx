/**
 * Text editor wrapper of HTML files: the browser bar at the top right (IDEA's
 * floating "open in browser" toolbar) and the built-in preview beside the
 * editor, served by the built-in server and reloaded when files are saved.
 */
import { useEffect, useState, type ReactNode } from "react";
import clsx from "clsx";
import { onIdeEvent } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { IconButton } from "@/ui";
import { browserIcon, openInBrowser, useBrowsers, useWebPreview, webUrl } from "./browsers";
import styles from "./Web.module.css";
import { tx } from "@/platform/i18n";

const RELOAD_DELAY = 250;

export function WebEditor({ path, children }: { path: string; tabId: string; children: ReactNode }) {
  const preview = useWebPreview((s) => !!s[path]);
  const browsers = useBrowsers((s) => s.list);
  return (
    <div className={clsx(styles.split, preview && styles.withPreview)}>
      <div className={styles.editor}>
        {children}
        <div className={styles.bar} role="toolbar" aria-label={tx("Open in browser")}>
          <IconButton
            icon="eye"
            size="small"
            action="web.togglePreview"
            actionArg={path}
            pressed={preview}
            title="Built-in Preview"
          />
          {browsers.map((b) => (
            <IconButton
              key={b.id}
              icon={browserIcon(b.id)}
              size="small"
              title={b.name}
              onClick={() => void openInBrowser(path, b.id)}
            />
          ))}
        </div>
      </div>
      {preview && <WebPreview path={path} />}
    </div>
  );
}

function WebPreview({ path }: { path: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [seq, setSeq] = useState(0);
  const root = useProject((s) => s.project?.root ?? null);

  useEffect(() => {
    let live = true;
    webUrl(path).then(
      (u) => live && setUrl(u),
      (e: unknown) => live && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      live = false;
    };
  }, [path]);

  // Saved edits anywhere in the project (scripts, styles) reload the page.
  useEffect(() => {
    const scope = root ?? path.slice(0, path.lastIndexOf("/"));
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reload = () => {
      clearTimeout(timer);
      timer = setTimeout(() => setSeq((n) => n + 1), RELOAD_DELAY);
    };
    const offFs = onIdeEvent("fsChanged", (e) => {
      if ([...e.created, ...e.modified, ...e.removed].some((p) => p === scope || p.startsWith(scope + "/"))) reload();
    });
    const offDirty = onIdeEvent("docDirty", (e) => {
      if (!e.dirty) reload();
    });
    return () => {
      clearTimeout(timer);
      offFs();
      offDirty();
    };
  }, [path, root]);

  if (error) return <div className={clsx(styles.preview, styles.message)}>{error}</div>;
  if (!url) return <div className={styles.preview} />;
  return (
    <iframe
      key={seq}
      className={styles.preview}
      title={tx("HTML preview")}
      src={url}
      sandbox="allow-scripts allow-same-origin allow-forms allow-modals allow-popups"
    />
  );
}
