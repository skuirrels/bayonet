/**
 * NuGet tool window: search the package sources, see what each project references,
 * and install, update or remove packages per project or across the solution.
 * Everything goes through `dotnet package`, so `nuget.config`, central package
 * management and credential providers apply as on the command line.
 */
import { useEffect, useMemo } from "react";
import clsx from "clsx";
import { openExternalUrl } from "@/platform/openUrl";
import { tx } from "@/platform/i18n";
import {
  Badge,
  Button,
  Checkbox,
  EmptyState,
  Icon,
  IconButton,
  List,
  SearchField,
  Select,
  Spinner,
  SplitGroup,
  SplitHandle,
  SplitPanel,
  T,
} from "@/ui";
import {
  formatDownloads,
  installedPackages,
  projectAction,
  projectsOf,
  searchProblems,
  searchRows,
  type InstalledPackage,
  type ProjectRef,
  type ProjectUse,
} from "./model";
import {
  checkUpdates,
  installPackage,
  loadInstalled,
  removePackage,
  runSearch,
  selectPackage,
  setPrerelease,
  setQuery,
  setVersion,
  useNuget,
} from "./store";
import type { PackageSummary } from "@/bindings";
import styles from "./NugetView.module.css";

type Row =
  | { kind: "section"; key: string; label: string; busy?: boolean }
  | { kind: "installed"; key: string; pkg: InstalledPackage }
  | {
      kind: "found";
      key: string;
      pkg: PackageSummary & { source: string };
      installed: InstalledPackage | undefined;
    }
  | { kind: "more"; key: string };

export function NugetView() {
  const s = useNuget();
  useEffect(() => {
    if (!useNuget.getState().installed) void loadInstalled();
  }, []);
  const installed = useMemo(() => installedPackages(s.installed, s.outdated), [s.installed, s.outdated]);
  const projects = useMemo(() => projectsOf(s.installed), [s.installed]);
  const found = useMemo(() => searchRows(s.search), [s.search]);
  const problems = useMemo(
    () => [...searchProblems(s.search), ...(s.installed?.problems ?? [])],
    [s.search, s.installed],
  );
  const q = s.query.trim().toLowerCase();
  const rows = useMemo<Row[]>(() => {
    const byId = new Map(installed.map((p) => [p.id.toLowerCase(), p]));
    const mine = installed.filter((p) => !q || p.id.toLowerCase().includes(q));
    const count = mine.length;
    const out: Row[] = [
      {
        kind: "section",
        key: "s:installed",
        label: tx`Installed (${count})`,
        busy: s.loading,
      },
      ...mine.map((pkg): Row => ({
        kind: "installed",
        key: `i:${pkg.id}`,
        pkg,
      })),
    ];
    if (q) {
      out.push({
        kind: "section",
        key: "s:found",
        label: tx`Search Results`,
        busy: s.searching,
      });
      for (const pkg of found)
        out.push({
          kind: "found",
          key: `f:${pkg.id}`,
          pkg,
          installed: byId.get(pkg.id.toLowerCase()),
        });
      if (s.more && !s.searching) out.push({ kind: "more", key: "more" });
    }
    return out;
  }, [installed, found, q, s.loading, s.searching, s.more]);
  const active = rows.findIndex(
    (r) => (r.kind === "installed" || r.kind === "found") && r.pkg.id.toLowerCase() === s.selected?.toLowerCase(),
  );
  const selectedInstalled = installed.find((p) => p.id.toLowerCase() === s.selected?.toLowerCase());
  const selectedFound = found.find((p) => p.id.toLowerCase() === s.selected?.toLowerCase());

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <SearchField
          className={styles.search}
          value={s.query}
          onChange={setQuery}
          placeholder="Search packages"
          aria-label="Search packages"
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch(false);
          }}
        />
        <Checkbox checked={s.prerelease} onChange={setPrerelease} label={<T>Prerelease</T>} />
        <IconButton
          icon="refresh"
          title="Reload Installed Packages"
          onClick={() => void loadInstalled()}
          disabled={s.loading}
        />
        <IconButton
          icon="arrow-up"
          title="Check for Updates"
          onClick={() => void checkUpdates()}
          disabled={s.checking || !s.installed}
        />
        {(s.checking || s.searching || s.loading) && <Spinner size={14} />}
      </div>
      {(s.error || problems.length > 0) && (
        <div className={styles.banner} role="alert">
          {[s.error, ...problems].filter(Boolean).join("\n")}
        </div>
      )}
      <SplitGroup orientation="horizontal" id="nuget-split" className={styles.split}>
        <SplitPanel id="packages" minSize={200} defaultSize="45%" className={styles.panel}>
          <List<Row>
            aria-label="Packages"
            items={rows}
            getKey={(r) => r.key}
            isSection={(r) => r.kind === "section"}
            renderSection={(r) =>
              r.kind === "section" ? (
                <div className={styles.section}>
                  {r.label}
                  {r.busy && <Spinner size={12} />}
                </div>
              ) : null
            }
            activeIndex={active}
            onActiveChange={(i) => {
              const r = rows[i];
              if (r?.kind === "installed" || r?.kind === "found") selectPackage(r.pkg.id);
            }}
            onActivate={(r) => {
              if (r.kind === "more") void runSearch(true);
            }}
            rowHeight={40}
            sectionHeight={24}
            renderItem={(r) => <PackageRow row={r} />}
            empty={<EmptyState title={s.loading ? "Loading packages…" : "No packages"} />}
          />
        </SplitPanel>
        <SplitHandle />
        <SplitPanel id="details" minSize={240} className={styles.panel}>
          {s.selected ? (
            <Details
              id={selectedInstalled?.id ?? selectedFound?.id ?? s.selected}
              summary={selectedFound}
              installed={selectedInstalled}
              projects={projects}
            />
          ) : (
            <EmptyState title="Select a package" />
          )}
        </SplitPanel>
      </SplitGroup>
    </div>
  );
}

function PackageRow({ row }: { row: Row }) {
  if (row.kind === "more")
    return (
      <div className={clsx(styles.row, styles.more)}>
        <T>Load more results</T>
      </div>
    );
  if (row.kind === "section") return null;
  const id = row.pkg.id;
  const version = row.pkg.version;
  const update = row.kind === "installed" ? row.pkg.update : row.installed?.update;
  const detail =
    row.kind === "installed" ? row.pkg.uses.map((u) => u.name).join(", ") : (row.pkg.description ?? "").split("\n")[0];
  return (
    <div className={styles.row}>
      <Icon name="package" size={16} className={styles.rowIcon} />
      <div className={styles.rowText}>
        <div className={styles.rowTitle}>
          <span className={styles.id}>{id}</span>
          <span className={styles.version}>{version}</span>
          {update && <Badge tone="accent">{`↑ ${update}`}</Badge>}
          {row.kind === "found" && row.installed && <Badge>{tx`installed`}</Badge>}
          {row.kind === "found" && row.pkg.totalDownloads != null && (
            <span className={styles.downloads}>{formatDownloads(row.pkg.totalDownloads)}</span>
          )}
        </div>
        <div className={styles.rowDetail}>{detail}</div>
      </div>
    </div>
  );
}

function Details({
  id,
  summary,
  installed,
  projects,
}: {
  id: string;
  summary: (PackageSummary & { source: string }) | undefined;
  installed: InstalledPackage | undefined;
  projects: ProjectRef[];
}) {
  const { versions, version, busy } = useNuget();
  const central = [...new Set(projects.map((p) => p.central).filter((c): c is string => !!c))];
  const usesOf = (p: ProjectRef): ProjectUse | undefined => installed?.uses.find((u) => u.project === p.path);
  const missing = projects.filter((p) => !usesOf(p));
  const updatable = projects.filter((p) => {
    const a = projectAction(usesOf(p), version);
    return a === "update" || a === "downgrade";
  });
  const options = (versions ?? []).map((v) => ({ value: v, label: v }));
  return (
    <div className={styles.details}>
      <div className={styles.detailsHead}>
        <Icon name="package" size={20} />
        <span className={styles.detailsId}>{id}</span>
        {summary?.projectUrl && (
          <IconButton
            icon="external-link"
            title="Open Project Website"
            onClick={() => void openExternalUrl(summary.projectUrl!)}
          />
        )}
      </div>
      {summary?.description && <p className={styles.description}>{summary.description}</p>}
      <dl className={styles.facts}>
        {summary?.owners && (
          <>
            <dt>
              <T>Owners</T>
            </dt>
            <dd>{summary.owners}</dd>
          </>
        )}
        {summary?.totalDownloads != null && (
          <>
            <dt>
              <T>Downloads</T>
            </dt>
            <dd>{formatDownloads(summary.totalDownloads)}</dd>
          </>
        )}
        {summary?.source && (
          <>
            <dt>
              <T>Source</T>
            </dt>
            <dd>{summary.source}</dd>
          </>
        )}
        {central.length > 0 && (
          <>
            <dt>
              <T>Versions in</T>
            </dt>
            <dd title={central.join("\n")}>{central.map((c) => c.split(/[\\/]/).pop()).join(", ")}</dd>
          </>
        )}
      </dl>
      <div className={styles.versionBar}>
        <span>
          <T>Version</T>
        </span>
        {versions === null ? (
          <Spinner size={14} />
        ) : (
          <Select
            className={styles.versionSelect}
            value={version}
            options={options}
            onChange={setVersion}
            searchable={options.length > 12}
            aria-label="Version"
          />
        )}
        {missing.length > 1 && (
          <Button
            size="small"
            variant="primary"
            disabled={!version || busy.size > 0}
            onClick={() =>
              void installPackage(
                missing.map((p) => p.path),
                id,
                version,
              )
            }
          >
            {tx`Install in all projects`}
          </Button>
        )}
        {updatable.length > 1 && (
          <Button
            size="small"
            disabled={!version || busy.size > 0}
            onClick={() =>
              void installPackage(
                updatable.map((p) => p.path),
                id,
                version,
              )
            }
          >
            {tx`Update all to ${version ?? ""}`}
          </Button>
        )}
      </div>
      <div className={styles.projects} role="table" aria-label="Projects">
        {projects.map((p) => {
          const use = usesOf(p);
          const action = projectAction(use, version);
          const working = busy.has(p.path);
          return (
            <div key={p.path} className={styles.projectRow} role="row">
              <Icon name="module" size={16} className={styles.rowIcon} />
              <span className={styles.projectName} role="cell" title={p.path}>
                {p.name}
              </span>
              <span className={styles.projectVersion} role="cell">
                {use ? use.version : "—"}
              </span>
              <span className={styles.projectActions} role="cell">
                {working ? (
                  <Spinner size={14} />
                ) : (
                  <>
                    {action === "install" && (
                      <IconButton
                        icon="plus"
                        title={tx`Install in ${p.name}`}
                        disabled={!version}
                        onClick={() => void installPackage([p.path], id, version)}
                      />
                    )}
                    {(action === "update" || action === "downgrade") && (
                      <IconButton
                        icon={action === "update" ? "arrow-up" : "arrow-down"}
                        title={action === "update" ? tx`Update to ${version ?? ""}` : tx`Downgrade to ${version ?? ""}`}
                        onClick={() => void installPackage([p.path], id, version)}
                      />
                    )}
                    {use && !use.autoReferenced && (
                      <IconButton
                        icon="minus"
                        tone="danger"
                        title={tx`Remove from ${p.name}`}
                        onClick={() => void removePackage([p.path], id)}
                      />
                    )}
                  </>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
