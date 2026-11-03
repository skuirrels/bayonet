/**
 * New Project: every installed `dotnet new` project template (custom ones too),
 * its options, and whether to add the result to the solution (SLN-7).
 */
import { useEffect, useMemo, useState } from "react";
import { commands } from "@/bindings";
import type { ProjectTemplate, TemplateParameter } from "@/bindings";
import { call } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { useProject } from "@/platform/project";
import { editorService, notifications } from "@/workbench/contributions";
import { Button, Checkbox, Dialog, EmptyState, Input, List, SearchField, Select, Spinner, T } from "@/ui";
import { closeDialog, useSolution } from "./store";
import styles from "./Dialogs.module.css";

/** Template order: common ones first, then by name. */
const COMMON = [
  "console",
  "classlib",
  "webapi",
  "web",
  "worker",
  "xunit",
  "nunit",
  "mstest",
  "blazor",
  "aspire-apphost",
];

export function matchesTemplate(t: ProjectTemplate, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [t.name, t.shortName, t.description ?? "", ...t.tags].some((s) => s.toLowerCase().includes(q));
}

export function sortTemplates(list: readonly ProjectTemplate[]): ProjectTemplate[] {
  const rank = (t: ProjectTemplate) => {
    const i = COMMON.indexOf(t.shortName);
    return i < 0 ? COMMON.length : i;
  };
  return [...list].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

/** A valid project name: a .NET identifier-ish file name. */
export const validName = (name: string) => /^[A-Za-z_][\w.-]*$/.test(name.trim());

let cached: ProjectTemplate[] | null = null;

export function NewProjectDialog({
  directory,
  solutionFolder = null,
}: {
  directory: string | null;
  solutionFolder?: string | null;
}) {
  const root = useProject((s) => s.project?.root ?? "");
  const solution = useSolution((s) => s.solution);
  const [templates, setTemplates] = useState<ProjectTemplate[] | null>(cached);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [name, setName] = useState("");
  // Solution folders usually mirror directories (`src`, `tests`), so a folder's projects go there.
  const [location, setLocation] = useState(directory ?? (root ? `${root}/${solutionFolder ?? "src"}` : ""));
  const [language, setLanguage] = useState<string | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [add, setAdd] = useState(!!solution);
  const [folder, setFolder] = useState(solutionFolder ?? "");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (cached) return;
    call(commands.dotnetTemplates())
      .then((t) => {
        cached = t.filter((x) => x.kind === "project");
        setTemplates(cached);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const shown = useMemo(
    () => sortTemplates((templates ?? []).filter((t) => matchesTemplate(t, query))),
    [templates, query],
  );
  const template = shown.find((t) => t.id === selected) ?? shown[0] ?? null;
  const lang = language && template?.languages.includes(language) ? language : (template?.languages[0] ?? null);
  const valueOf = (p: TemplateParameter) => values[`${template?.id}\u0000${p.option}`] ?? p.defaultValue ?? "";
  const setValue = (p: TemplateParameter, v: string) =>
    setValues((s) => ({ ...s, [`${template?.id}\u0000${p.option}`]: v }));
  const output = name.trim() ? `${location.replace(/\/+$/, "")}/${name.trim()}` : "";
  const canCreate = !!template && validName(name) && !!location.trim() && !busy;

  const create = async () => {
    if (!template || !canCreate) return;
    setBusy(true);
    setError(null);
    const changed = template.parameters
      .filter((p) => valueOf(p) !== (p.defaultValue ?? ""))
      .map((p) => ({ option: p.option, value: valueOf(p) }));
    try {
      const project = await call(
        commands.dotnetNewProject(
          template.shortName,
          name.trim(),
          output,
          lang,
          changed,
          add && !!solution,
          folder.trim() || null,
        ),
      );
      const projectName = name.trim();
      notifications().notify({
        level: "info",
        title: tx`Created ${projectName}`,
        group: "tools",
      });
      closeDialog();
      void editorService().openFile(project, { focus: true });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const browse = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      defaultPath: location || root || undefined,
    });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (dir) setLocation(dir);
  };

  return (
    <Dialog
      open
      onClose={closeDialog}
      title="New Project"
      width={880}
      resizable="both"
      footerStart={busy ? <Spinner size={14} /> : undefined}
      buttons={[
        { label: "Cancel", role: "cancel" },
        {
          label: "Create",
          role: "default",
          disabled: !canCreate,
          onClick: () => void create(),
        },
      ]}
    >
      <div className={styles.newProject}>
        <div className={styles.templates}>
          <SearchField value={query} onChange={setQuery} placeholder="Search templates" aria-label="Search templates" />
          {templates === null && !error ? (
            <div className={styles.loading}>
              <Spinner size={14} /> <T>Loading templates…</T>
            </div>
          ) : (
            <List<ProjectTemplate>
              aria-label="Templates"
              className={styles.templateList}
              items={shown}
              getKey={(t) => t.id}
              activeIndex={template ? shown.indexOf(template) : -1}
              onActiveChange={(i) => setSelected(shown[i]?.id ?? null)}
              rowHeight={38}
              renderItem={(t) => (
                <div className={styles.templateRow}>
                  <span className={styles.templateName}>{t.name}</span>
                  <span className={styles.templateMeta}>
                    {t.shortName}
                    {t.tags.length ? ` · ${t.tags.join(", ")}` : ""}
                  </span>
                </div>
              )}
              empty={<EmptyState title="No templates" />}
            />
          )}
        </div>
        <div className={styles.form}>
          {template && (
            <>
              <div className={styles.templateTitle}>{template.name}</div>
              {template.description && <p className={styles.templateDescription}>{template.description}</p>}
            </>
          )}
          <label className={styles.field}>
            <span>
              <T>Name</T>
            </span>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              invalid={!!name && !validName(name)}
              aria-label="Project name"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter") void create();
              }}
            />
          </label>
          <label className={styles.field}>
            <span>
              <T>Location</T>
            </span>
            <span className={styles.row}>
              <Input value={location} onChange={(e) => setLocation(e.target.value)} aria-label="Location" mono />
              <Button size="small" onClick={() => void browse()}>
                {tx`Browse…`}
              </Button>
            </span>
          </label>
          {output && <div className={styles.hint}>{output}</div>}
          {template && template.languages.length > 1 && (
            <label className={styles.field}>
              <span>
                <T>Language</T>
              </span>
              <Select
                value={lang}
                options={template.languages.map((l) => ({
                  value: l,
                  label: l,
                }))}
                onChange={setLanguage}
                aria-label="Language"
              />
            </label>
          )}
          {template?.parameters.map((p) => (
            <ParameterField key={p.option} parameter={p} value={valueOf(p)} onChange={(v) => setValue(p, v)} />
          ))}
          {solution && (
            <>
              <Checkbox checked={add} onChange={setAdd} label={tx`Add to solution '${solution.name}'`} />
              {add && (
                <label className={styles.field}>
                  <span>
                    <T>Solution folder</T>
                  </span>
                  <Input value={folder} onChange={(e) => setFolder(e.target.value)} aria-label="Solution folder" />
                </label>
              )}
            </>
          )}
          {error && (
            <div className={styles.error} role="alert">
              {error}
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}

function ParameterField({
  parameter: p,
  value,
  onChange,
}: {
  parameter: TemplateParameter;
  value: string;
  onChange: (v: string) => void;
}) {
  if (p.dataType === "bool")
    return (
      <span title={p.description ?? undefined}>
        <Checkbox checked={value === "true"} onChange={(c) => onChange(c ? "true" : "false")} label={p.label} />
      </span>
    );
  return (
    <label className={styles.field} title={p.description ?? undefined}>
      <span>{p.label}</span>
      {p.dataType === "choice" && p.choices.length ? (
        <Select
          value={value || null}
          options={p.choices.map((c) => ({
            value: c.value,
            label: c.label === c.value ? c.value : `${c.label} (${c.value})`,
          }))}
          onChange={onChange}
          aria-label={p.label}
        />
      ) : (
        <Input value={value} onChange={(e) => onChange(e.target.value)} aria-label={p.label} />
      )}
    </label>
  );
}
