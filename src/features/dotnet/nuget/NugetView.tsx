/**
 * NuGet tool window: search the package sources, see what each project references,
 * and install, update or remove packages per project or across the solution.
 * Everything goes through `dotnet package`, so `nuget.config`, central package
 * management and credential providers apply as on the command line.
 */
import { useEffect, useMemo, useRef, useState } from "react";
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
  Menu,
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
  updatesOf,
  type InstalledPackage,
  type ProjectRef,
  type ProjectUse,
} from "./model";
import {
  checkAudit,
  checkUpdates,
  chooseAllFeeds,
  installPackage,
  loadInstalled,
  loadSources,
  removePackage,
  runSearch,
  selectPackage,
  setPrerelease,
  setQuery,
  setVersion,
  tickAll,
  tickPackage,
  toggleFeed,
  updatePackages,
  useNuget,
} from "./store";
import { FeedsDialog } from "./FeedsDialog";
import { auditIssues, reasonLabel, severityLabel, severityRank, type PackageIssues } from "./audit";
import type { PackageSummary } from "@/bindings";
import styles from "./NugetView.module.css";

type Row =
  | { kind: "section"; key: string; label: string; busy?: boolean }
  | { kind: "installed"; key: string; pkg: InstalledPackage; issues: PackageIssues | undefined }
  | { kind: "transitive"; key: string; pkg: { id: string; version: string }; issues: PackageIssues }
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
    if (!useNuget.getState().sources) void loadSources();
  }, []);
  const [managing, setManaging] = useState(false);
  const installed = useMemo(() => installedPackages(s.installed, s.outdated), [s.installed, s.outdated]);
  const projects = useMemo(() => projectsOf(s.installed), [s.installed]);
  const issues = useMemo(() => auditIssues(s.audit), [s.audit]);
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
        issues: issues.get(pkg.id.toLowerCase()),
      })),
    ];
    // Vulnerable packages no project references: fixed by referencing a patched version.
    const transitive = [...issues.values()]
      .filter((i) => i.transitive && i.vulnerabilities.length && (!q || i.id.toLowerCase().includes(q)))
      .sort((a, b) => severityRank(b.worst ?? "") - severityRank(a.worst ?? "") || a.id.localeCompare(b.id));
    if (transitive.length) {
      const n = transitive.length;
      out.push({ kind: "section", key: "s:transitive", label: tx`Vulnerable Transitive Packages (${n})` });
      for (const i of transitive)
        out.push({
          kind: "transitive",
          key: `t:${i.id}`,
          pkg: { id: i.id, version: [...new Set(i.projects.map((p) => p.version ?? ""))].join(", ") },
          issues: i,
        });
    }
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
  }, [installed, issues, found, q, s.loading, s.searching, s.more]);
  const active = rows.findIndex(
    (r) =>
      (r.kind === "installed" || r.kind === "found" || r.kind === "transitive") &&
      r.pkg.id.toLowerCase() === s.selected?.toLowerCase(),
  );
  const selectedInstalled = installed.find((p) => p.id.toLowerCase() === s.selected?.toLowerCase());
  // Packages Update All moves: an update known, in a project that can take it.
  const { updatable, updatableIds } = useMemo(() => {
    const ids = new Set(updatesOf(installed).map((u) => u.id.toLowerCase()));
    return { updatable: installed.filter((p) => ids.has(p.id.toLowerCase())), updatableIds: ids };
  }, [installed]);
  const tickedUpdatable = updatable.filter((p) => s.ticked.has(p.id.toLowerCase()));
  const working = s.updating !== null || s.busy.size > 0;
  const updatingDone = s.updating?.done ?? 0;
  const updatingTotal = s.updating?.total ?? 0;
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
        <FeedPicker onManage={() => setManaging(true)} />
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
        <IconButton
          icon="warning"
          title="Check for Vulnerable and Deprecated Packages"
          onClick={() => void checkAudit()}
          disabled={s.auditing || !s.installed}
        />
        {(s.checking || s.searching || s.loading || s.auditing) && <Spinner size={14} />}
        {updatable.length > 0 && (
          <div className={styles.updateBar}>
            {s.updating ? (
              <span className={styles.updating}>
                <Spinner size={12} />
                {tx`Updating ${updatingDone} of ${updatingTotal}…`}
              </span>
            ) : (
              <>
                <Checkbox
                  aria-label="Select All Updates"
                  checked={tickedUpdatable.length === updatable.length}
                  indeterminate={tickedUpdatable.length > 0 && tickedUpdatable.length < updatable.length}
                  onChange={(on) =>
                    tickAll(
                      updatable.map((p) => p.id),
                      on,
                    )
                  }
                />
                {tickedUpdatable.length > 0 && (
                  <Button size="small" disabled={working} onClick={() => void updatePackages(tickedUpdatable)}>
                    {tx`Update Selected (${tickedUpdatable.length})`}
                  </Button>
                )}
                <Button
                  size="small"
                  variant="primary"
                  disabled={working}
                  onClick={() => void updatePackages(updatable)}
                >
                  {tx`Update All (${updatable.length})`}
                </Button>
              </>
            )}
          </div>
        )}
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
              if (r?.kind === "installed" || r?.kind === "found" || r?.kind === "transitive") selectPackage(r.pkg.id);
            }}
            onActivate={(r) => {
              if (r.kind === "more") void runSearch(true);
            }}
            rowHeight={40}
            sectionHeight={24}
            renderItem={(r) => (
              <PackageRow
                row={r}
                tickable={r.kind === "installed" && updatableIds.has(r.pkg.id.toLowerCase())}
                ticked={r.kind === "installed" && s.ticked.has(r.pkg.id.toLowerCase())}
                tickDisabled={s.updating !== null}
              />
            )}
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
              issues={issues.get(s.selected.toLowerCase())}
              projects={projects}
            />
          ) : (
            <EmptyState title="Select a package" />
          )}
        </SplitPanel>
      </SplitGroup>
      {managing && <FeedsDialog onClose={() => setManaging(false)} />}
    </div>
  );
}

/** The feeds searched (Rider's feed picker): every enabled feed, or the ones ticked; Manage Feeds… edits them. */
function FeedPicker({ onManage }: { onManage: () => void }) {
  const sources = useNuget((s) => s.sources);
  const sourcesError = useNuget((s) => s.sourcesError);
  const feeds = useNuget((s) => s.feeds);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const enabled = (sources ?? []).filter((x) => x.enabled);
  const chosen = feeds ?? enabled.map((x) => x.name);
  const count = chosen.length;
  const label =
    sources === null
      ? sourcesError
        ? tx`No feeds`
        : tx`Feeds`
      : enabled.length === 0
        ? tx`No feeds`
        : feeds === null && enabled.length > 1
          ? tx`All feeds`
          : count === 1
            ? chosen[0]!
            : tx`${count} feeds`;
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className={styles.feedButton}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={tx`Package feeds: ${label}`}
        title={sourcesError ?? undefined}
        onClick={() => {
          if (useNuget.getState().sources === null) void loadSources();
          setOpen((o) => !o);
        }}
      >
        <span className={styles.feedLabel}>{label}</span>
        <Icon name="chevron-down" size={12} />
      </button>
      <Menu
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        aria-label="Package Feeds"
        items={[
          { kind: "header", label: "Enabled Feeds" },
          ...(enabled.length > 1
            ? [{ id: "all", label: "All Feeds", checked: feeds === null, run: chooseAllFeeds }]
            : []),
          ...enabled.map((x) => ({
            id: `feed:${x.name}`,
            label: x.name,
            verbatim: true,
            checked: chosen.includes(x.name),
            run: () => toggleFeed(x.name),
          })),
          { kind: "separator" as const },
          { id: "manage", label: "Manage Feeds…", icon: "settings", run: onManage },
        ]}
      />
    </>
  );
}

function PackageRow({
  row,
  tickable = false,
  ticked = false,
  tickDisabled = false,
}: {
  row: Row;
  tickable?: boolean;
  ticked?: boolean;
  tickDisabled?: boolean;
}) {
  if (row.kind === "more")
    return (
      <div className={clsx(styles.row, styles.more)}>
        <T>Load more results</T>
      </div>
    );
  if (row.kind === "section") return null;
  const id = row.pkg.id;
  const version = row.pkg.version;
  const update = row.kind === "installed" ? row.pkg.update : row.kind === "found" ? row.installed?.update : null;
  const detail =
    row.kind === "installed"
      ? row.pkg.uses.map((u) => u.name).join(", ")
      : row.kind === "transitive"
        ? row.issues.projects.map((p) => p.name).join(", ")
        : (row.pkg.description ?? "").split("\n")[0];
  const issues = row.kind === "installed" || row.kind === "transitive" ? row.issues : undefined;
  return (
    <div className={styles.row}>
      {row.kind === "installed" && (
        <span
          className={styles.tick}
          // The list selects and activates rows on these; the box only ticks.
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {tickable && (
            <Checkbox
              aria-label={tx`Select ${id} for update`}
              checked={ticked}
              disabled={tickDisabled}
              onChange={(on) => tickPackage(id, on)}
            />
          )}
        </span>
      )}
      <Icon name="package" size={16} className={styles.rowIcon} />
      <div className={styles.rowText}>
        <div className={styles.rowTitle}>
          <span className={styles.id}>{id}</span>
          <span className={styles.version}>{version}</span>
          {update && <Badge tone="accent">{`↑ ${update}`}</Badge>}
          {issues?.worst && (
            <Badge tone={severityRank(issues.worst) >= 2 ? "danger" : "warning"}>{tx`vulnerable`}</Badge>
          )}
          {issues?.deprecation && <Badge tone="warning">{tx`deprecated`}</Badge>}
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
  issues,
  projects,
}: {
  id: string;
  summary: (PackageSummary & { source: string }) | undefined;
  installed: InstalledPackage | undefined;
  issues: PackageIssues | undefined;
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
      {issues && <Issues issues={issues} />}
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

/** Advisories and the deprecation of the selected package. */
function Issues({ issues }: { issues: PackageIssues }) {
  const versions = [...new Set(issues.projects.map((p) => p.version).filter(Boolean))].join(", ");
  const names = issues.projects.map((p) => p.name).join(", ");
  const d = issues.deprecation;
  const reasons = d?.reasons.map(reasonLabel).join(", ") ?? "";
  const alternative = d?.alternative;
  return (
    <div className={styles.issues} role="alert">
      {issues.vulnerabilities.length > 0 && (
        <>
          <div className={styles.issuesTitle}>
            <Icon name="warning" size={14} />
            {issues.transitive
              ? tx`${versions} comes in under other packages of ${names} and has known vulnerabilities. Install a patched version to replace it.`
              : tx`${versions} has known vulnerabilities in ${names}. Update to a patched version.`}
          </div>
          <ul className={styles.advisories}>
            {issues.vulnerabilities.map((a) => (
              <li key={a.url}>
                <Badge tone={severityRank(a.severity) >= 2 ? "danger" : "warning"}>{severityLabel(a.severity)}</Badge>
                <button type="button" className={styles.advisory} onClick={() => void openExternalUrl(a.url)}>
                  {a.url.replace(/^https:\/\/github\.com\/advisories\//, "")}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      {d && (
        <div className={styles.issuesTitle}>
          <Icon name="warning" size={14} />
          {alternative ? tx`Deprecated (${reasons}): use ${alternative} instead.` : tx`Deprecated (${reasons}).`}
          {alternative && (
            <Button size="small" onClick={() => selectPackage(alternative)}>
              {tx`Show ${alternative}`}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
