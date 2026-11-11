/**
 * Manage Feeds: the package sources of the `nuget.config` hierarchy. Every change goes
 * through `dotnet nuget add / update / remove / enable / disable source`, so it lands in the
 * config file that defines the source, as on the command line.
 */
import { useEffect, useState } from "react";
import clsx from "clsx";
import { tx } from "@/platform/i18n";
import { isRemoteWindow } from "@/platform/remote";
import { confirm } from "@/workbench/messages";
import { Button, Checkbox, Dialog, IconButton, Input, Select, Spinner, T } from "@/ui";
import type { PackageSource, SourceScope } from "@/bindings";
import { addSource, enableSource, loadSources, removeSource, updateSource, useNuget } from "./store";
import styles from "./NugetView.module.css";

const SCOPES: Array<{ value: SourceScope; label: string }> = [
  { value: "user", label: "User settings (every solution)" },
  { value: "solution", label: "This solution (nuget.config)" },
];

export function FeedsDialog({ onClose }: { onClose: () => void }) {
  const sources = useNuget((s) => s.sources);
  const sourcesError = useNuget((s) => s.sourcesError);
  // Config files may have changed outside the IDE since the last listing.
  useEffect(() => void loadSources(), []);
  // The source being edited; null: a new one.
  const [editing, setEditing] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [scope, setScope] = useState<SourceScope>("user");
  const [busy, setBusy] = useState(false);

  const edited = sources?.find((x) => x.name === editing) ?? null;
  const taken = editing === null && !!sources?.some((x) => x.name.toLowerCase() === name.trim().toLowerCase());
  const changed = edited ? url.trim() !== edited.url : true;
  const ok = !!name.trim() && !!url.trim() && !taken && changed && !busy;

  const run = async (op: () => Promise<boolean>) => {
    setBusy(true);
    try {
      return await op();
    } finally {
      setBusy(false);
    }
  };
  const startNew = () => {
    setEditing(null);
    setName("");
    setUrl("");
  };
  const edit = (x: PackageSource) => {
    setEditing(x.name);
    setName(x.name);
    setUrl(x.url);
  };
  const save = async () => {
    if (!ok) return;
    const done = await run(() => (edited ? updateSource(edited.name, url) : addSource(name, url, scope)));
    if (done && !edited) startNew();
  };
  const remove = async (x: PackageSource) => {
    const yes = await confirm({
      title: tx`Remove Feed`,
      message: tx`Remove the package feed ${x.name} from its nuget.config?`,
      ok: tx`Remove`,
      icon: "warning",
    });
    if (!yes) return;
    if ((await run(() => removeSource(x.name))) && editing === x.name) startNew();
  };
  const browse = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false, title: tx`Local Package Folder` });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (path) setUrl(path);
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={<T>Manage Feeds</T>}
      width={560}
      buttons={[{ label: "Close", role: "cancel" }]}
      footerStart={busy ? <Spinner size={14} /> : undefined}
    >
      <div className={styles.feeds}>
        <div className={styles.feedList} role="listbox" aria-label="Package Feeds">
          {sources === null ? (
            <div className={clsx(styles.feedRow, sourcesError && styles.feedNote)}>
              {sourcesError ?? <Spinner size={14} />}
            </div>
          ) : sources.length === 0 ? (
            <div className={clsx(styles.feedRow, styles.feedNote)}>
              <T>No package feeds are configured.</T>
            </div>
          ) : (
            sources.map((x) => (
              <div
                key={x.name}
                role="option"
                aria-selected={editing === x.name}
                className={clsx(styles.feedRow, !x.enabled && styles.feedDisabled)}
                onClick={() => edit(x)}
              >
                <span onClick={(e) => e.stopPropagation()}>
                  <Checkbox
                    aria-label={tx`Enable ${x.name}`}
                    checked={x.enabled}
                    disabled={busy}
                    onChange={(on) => void run(() => enableSource(x.name, on))}
                  />
                </span>
                <span className={styles.feedText}>
                  <span className={styles.feedName}>{x.name}</span>
                  <span className={styles.feedUrl} title={x.url}>
                    {x.url}
                  </span>
                </span>
                <span className={styles.feedTools}>
                  <IconButton
                    icon="edit"
                    title={tx`Edit ${x.name}`}
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      edit(x);
                    }}
                  />
                  <IconButton
                    icon="minus"
                    tone="danger"
                    title={tx`Remove ${x.name}`}
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void remove(x);
                    }}
                  />
                </span>
              </div>
            ))
          )}
        </div>
        <form
          className={styles.feedForm}
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label htmlFor="nuget-feed-name">
            <T>Name</T>
          </label>
          <Input
            id="nuget-feed-name"
            data-autofocus
            value={name}
            disabled={edited !== null}
            onChange={(e) => setName(e.target.value)}
          />
          <span />
          <label htmlFor="nuget-feed-url">
            <T>URL or folder</T>
          </label>
          <Input
            id="nuget-feed-url"
            value={url}
            placeholder="https://api.nuget.org/v3/index.json"
            onChange={(e) => setUrl(e.target.value)}
          />
          {/* A remote project's feeds live on its host: a local folder means nothing there. */}
          {isRemoteWindow() ? (
            <span />
          ) : (
            <IconButton icon="folder" title="Choose Folder" onClick={() => void browse()} />
          )}
          {edited === null && (
            <>
              <span>
                <T>Save to</T>
              </span>
              <Select<SourceScope> aria-label="Save to" value={scope} options={SCOPES} onChange={setScope} />
              <span />
            </>
          )}
          <div className={styles.feedFormActions}>
            {taken && (
              <span className={styles.feedNote}>
                <T>A feed with this name exists.</T>
              </span>
            )}
            {edited !== null && (
              <Button size="small" onClick={startNew}>
                <T>New Feed</T>
              </Button>
            )}
            <Button size="small" variant="primary" type="submit" disabled={!ok}>
              {edited ? tx`Save` : tx`Add Feed`}
            </Button>
          </div>
        </form>
        <div className={styles.feedNote}>
          <T>Credentials for private feeds come from your NuGet credential providers and nuget.config.</T>
        </div>
      </div>
    </Dialog>
  );
}
