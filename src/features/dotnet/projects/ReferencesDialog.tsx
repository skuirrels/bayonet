/** Project References: tick the solution's projects that a project references (SLN-4). */
import { useEffect, useMemo, useState } from "react";
import { commands } from "@/bindings";
import { call } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { Checkbox, Dialog, EmptyState, Spinner, T } from "@/ui";
import { closeDialog, editReferences, useSolution, type SolutionProject } from "./store";
import styles from "./Dialogs.module.css";

/** What to add and remove to go from `before` to `after`. */
export function referenceChanges(before: ReadonlySet<string>, after: ReadonlySet<string>) {
  return {
    add: [...after].filter((p) => !before.has(p)),
    remove: [...before].filter((p) => !after.has(p)),
  };
}

export function ReferencesDialog({ project }: { project: SolutionProject }) {
  const name = project.name;
  const all = useSolution((s) => s.projects);
  const projects = useMemo(() => all.filter((p) => p.path !== project.path), [all, project.path]);
  const [before, setBefore] = useState<Set<string> | null>(null);
  const [after, setAfter] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    call(commands.dotnetProjectReferences(project.path))
      .then((refs) => {
        setBefore(new Set(refs));
        setAfter(new Set(refs));
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [project.path]);
  const changes = before ? referenceChanges(before, after) : { add: [], remove: [] };
  const dirty = changes.add.length + changes.remove.length > 0;
  const apply = async () => {
    setBusy(true);
    if (await editReferences(project, changes.add, changes.remove)) closeDialog();
    else setBusy(false);
  };
  return (
    <Dialog
      open
      onClose={closeDialog}
      title={tx`Project References: ${name}`}
      width={480}
      footerStart={busy ? <Spinner size={14} /> : undefined}
      buttons={[
        { label: "Cancel", role: "cancel" },
        {
          label: "OK",
          role: "default",
          disabled: !dirty || busy,
          onClick: () => void apply(),
        },
      ]}
    >
      <div className={styles.references}>
        {error && (
          <div className={styles.error} role="alert">
            {error}
          </div>
        )}
        {before === null && !error ? (
          <Spinner size={14} />
        ) : projects.length === 0 ? (
          <EmptyState title="No other projects in the solution" />
        ) : (
          projects.map((p) => (
            <Checkbox
              key={p.path}
              checked={after.has(p.path)}
              onChange={(on) =>
                setAfter((s) => {
                  const next = new Set(s);
                  if (on) next.add(p.path);
                  else next.delete(p.path);
                  return next;
                })
              }
              label={
                <span>
                  {p.name}
                  {p.folders.length > 0 && <span className={styles.hint}>{` ${p.folders.join("/")}`}</span>}
                </span>
              }
            />
          ))
        )}
        <p className={styles.hint}>
          <T>References are written with dotnet reference, which keeps the project file's formatting.</T>
        </p>
      </div>
    </Dialog>
  );
}
