/** NuGet tool window state: the search, the installed packages and the selected package. */
import { create } from "zustand";
import { commands } from "@/bindings";
import type { InstalledPackages, PackageAudit, PackageSearch, PackageSource, SourceScope } from "@/bindings";
import { call, isTauri } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { notifications } from "@/workbench/contributions";
import { chosenSources, isPrerelease, updatesOf, type InstalledPackage } from "./model";
import { publishAudit } from "./audit";

/** Results per page, as `dotnet package search` defaults to. */
export const PAGE = 20;
const SEARCH_DELAY = 350;

export interface NugetState {
  query: string;
  prerelease: boolean;
  search: PackageSearch | null;
  searching: boolean;
  /** More results may follow (the last page was full). */
  more: boolean;
  installed: InstalledPackages | null;
  /** The same listing with `--outdated`: latest versions. */
  outdated: InstalledPackages | null;
  loading: boolean;
  checking: boolean;
  /** Vulnerable and deprecated packages (asks the sources). */
  audit: PackageAudit | null;
  auditing: boolean;
  /** Selected package id. */
  selected: string | null;
  versions: string[] | null;
  /** The version the details pane installs or updates to. */
  version: string | null;
  /** Projects an install / update / remove is running for. */
  busy: ReadonlySet<string>;
  /** Installed packages ticked for Update Selected, by lower-case id. */
  ticked: ReadonlySet<string>;
  /** Packages updated so far by Update All / Update Selected, while it runs. */
  updating: { done: number; total: number } | null;
  /** The package sources of the `nuget.config` hierarchy; null until listed. */
  sources: PackageSource[] | null;
  /** Why listing the sources failed; they are listed again on the next look. */
  sourcesError: string | null;
  /** Names of the enabled sources searched; null: all of them. */
  feeds: string[] | null;
  error: string | null;
}

const initial = (): NugetState => ({
  query: "",
  prerelease: false,
  search: null,
  searching: false,
  more: false,
  installed: null,
  outdated: null,
  loading: false,
  checking: false,
  audit: null,
  auditing: false,
  selected: null,
  versions: null,
  version: null,
  busy: new Set(),
  ticked: new Set(),
  updating: null,
  sources: null,
  sourcesError: null,
  feeds: null,
  error: null,
});

export const useNuget = create<NugetState>(initial);

export function resetNuget() {
  clearTimeout(searchTimer);
  seq.search++;
  seq.versions++;
  seq.installed++;
  seq.audit++;
  seq.sources++;
  publishAudit(null);
  useNuget.setState(initial());
}

const seq = { search: 0, versions: 0, installed: 0, audit: 0, sources: 0 };
let searchTimer: ReturnType<typeof setTimeout> | undefined;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** `--source` values for the commands: the chosen feeds' URLs, none when every enabled feed is chosen. */
const feedArgs = () => {
  const { sources, feeds } = useNuget.getState();
  return chosenSources(sources, feeds);
};

/** Re-reads the packages of every project; `outdated` then asks the sources for updates. */
export async function loadInstalled(outdated = false): Promise<void> {
  if (!isTauri()) return;
  const id = ++seq.installed;
  useNuget.setState({ loading: true, error: null });
  try {
    const installed = await call(commands.nugetInstalled(false, false, []));
    if (id !== seq.installed) return;
    useNuget.setState((s) => ({ installed, loading: false, ticked: keepTicked(s.ticked, installed) }));
    void checkAudit();
  } catch (e) {
    if (id === seq.installed) useNuget.setState({ loading: false, error: message(e) });
    return;
  }
  if (outdated || useNuget.getState().outdated) await checkUpdates();
}

/** Latest versions of the installed packages (needs the package sources). */
export async function checkUpdates(): Promise<void> {
  if (!isTauri()) return;
  const id = seq.installed;
  useNuget.setState({ checking: true });
  try {
    const outdated = await call(commands.nugetInstalled(true, useNuget.getState().prerelease, feedArgs()));
    if (id === seq.installed) useNuget.setState({ outdated, checking: false });
  } catch (e) {
    if (id === seq.installed) useNuget.setState({ checking: false, error: message(e) });
  }
}

/** Asks the sources which packages in use are vulnerable or deprecated, and warns in the project files. */
export async function checkAudit(): Promise<void> {
  if (!isTauri()) return;
  const id = ++seq.audit;
  useNuget.setState({ auditing: true });
  try {
    const audit = await call(commands.nugetAudit());
    if (id !== seq.audit) return;
    useNuget.setState({ audit, auditing: false });
    publishAudit(audit);
  } catch (e) {
    // Offline or no sources: keep what the last check found.
    if (id === seq.audit) useNuget.setState({ auditing: false });
    console.warn("nuget: audit failed", e);
  }
}

export function setQuery(query: string) {
  useNuget.setState({ query });
  clearTimeout(searchTimer);
  if (!query.trim()) {
    seq.search++;
    useNuget.setState({ search: null, searching: false, more: false });
    return;
  }
  searchTimer = setTimeout(() => void runSearch(false), SEARCH_DELAY);
}

export function setPrerelease(prerelease: boolean) {
  useNuget.setState({ prerelease, outdated: null });
  const { query, selected } = useNuget.getState();
  if (query.trim()) void runSearch(false);
  if (selected) void loadVersions(selected);
}

/** Searches, or with `append` fetches the next page. */
export async function runSearch(append: boolean): Promise<void> {
  const { query, prerelease, search } = useNuget.getState();
  if (!query.trim() || !isTauri()) return;
  const id = ++seq.search;
  const skip = append ? (search?.sources.reduce((n, s) => Math.max(n, s.packages.length), 0) ?? 0) : 0;
  useNuget.setState({ searching: true, error: null });
  try {
    const page = await call(commands.nugetSearch(query, prerelease, skip, PAGE, feedArgs()));
    if (id !== seq.search) return;
    const more = page.sources.some((s) => s.packages.length >= PAGE);
    const merged: PackageSearch =
      append && search
        ? {
            problems: page.problems,
            sources: page.sources.map((s) => ({
              ...s,
              packages: [...(search.sources.find((o) => o.source === s.source)?.packages ?? []), ...s.packages],
            })),
          }
        : page;
    useNuget.setState({ search: merged, searching: false, more });
  } catch (e) {
    if (id === seq.search) useNuget.setState({ searching: false, error: message(e) });
  }
}

export function selectPackage(id: string | null) {
  if (id === useNuget.getState().selected) return;
  useNuget.setState({ selected: id, versions: null, version: null });
  if (id) void loadVersions(id);
}

async function loadVersions(pkg: string) {
  if (!isTauri()) return;
  const id = ++seq.versions;
  try {
    const versions = await call(commands.nugetVersions(pkg, useNuget.getState().prerelease, feedArgs()));
    if (id !== seq.versions) return;
    const keep = useNuget.getState().version;
    const stable = versions.find((v) => !isPrerelease(v)) ?? versions[0] ?? null;
    useNuget.setState({
      versions,
      version: keep && versions.includes(keep) ? keep : stable,
    });
  } catch (e) {
    if (id === seq.versions) useNuget.setState({ versions: [], error: message(e) });
  }
}

export function setVersion(version: string) {
  useNuget.setState({ version });
}

async function change(projects: readonly string[], op: () => Promise<unknown>, done: string): Promise<boolean> {
  useNuget.setState((s) => ({
    busy: new Set([...s.busy, ...projects]),
    error: null,
  }));
  try {
    await op();
    notifications().notify({ level: "info", title: done, group: "tools" });
    return true;
  } catch (e) {
    notifications().notify({
      level: "error",
      title: tx`NuGet operation failed`,
      body: message(e),
      group: "tools",
    });
    return false;
  } finally {
    useNuget.setState((s) => ({
      busy: new Set([...s.busy].filter((p) => !projects.includes(p))),
    }));
    await loadInstalled();
  }
}

/** Installs package `id` at `version` into `projects`, or moves them to it. */
export function installPackage(projects: readonly string[], id: string, version: string | null): Promise<boolean> {
  const label = version ? `${id} ${version}` : id;
  const count = projects.length;
  return change(
    projects,
    () => call(commands.nugetInstall([...projects], id, version)),
    count === 1 ? tx`Installed ${label}` : tx`Installed ${label} into ${count} projects`,
  );
}

export function removePackage(projects: readonly string[], id: string): Promise<boolean> {
  return change(projects, () => call(commands.nugetRemove([...projects], id)), tx`Removed ${id}`);
}

/** Ticks of packages still installed. */
function keepTicked(ticked: ReadonlySet<string>, installed: InstalledPackages): ReadonlySet<string> {
  const ids = new Set(
    installed.projects.flatMap((p) => p.frameworks.flatMap((f) => f.packages.map((x) => x.id.toLowerCase()))),
  );
  const kept = [...ticked].filter((id) => ids.has(id));
  return kept.length === ticked.size ? ticked : new Set(kept);
}

/** Ticks or unticks installed package `id` for Update Selected. */
export function tickPackage(id: string, on: boolean) {
  useNuget.setState((s) => {
    const key = id.toLowerCase();
    if (s.ticked.has(key) === on) return s;
    const ticked = new Set(s.ticked);
    if (on) ticked.add(key);
    else ticked.delete(key);
    return { ticked };
  });
}

/** Ticks every package in `ids`, or unticks them all when `on` is false. */
export function tickAll(ids: readonly string[], on: boolean) {
  useNuget.setState({ ticked: on ? new Set(ids.map((id) => id.toLowerCase())) : new Set() });
}

/**
 * Moves each of `packages` to its newest version in every project that uses an older one,
 * one package at a time, then lists the packages once. Failures are reported together.
 */
export async function updatePackages(packages: readonly InstalledPackage[]): Promise<void> {
  const plan = updatesOf(packages);
  if (!plan.length || useNuget.getState().updating || !isTauri()) return;
  const projects = [...new Set(plan.flatMap((u) => u.projects))];
  useNuget.setState((s) => ({
    busy: new Set([...s.busy, ...projects]),
    updating: { done: 0, total: plan.length },
    error: null,
  }));
  const failed: string[] = [];
  const updated: string[] = [];
  try {
    for (const u of plan) {
      try {
        await call(commands.nugetInstall(u.projects, u.id, u.version));
        updated.push(u.id.toLowerCase());
      } catch (e) {
        failed.push(`${u.id} ${u.version}: ${message(e)}`);
      }
      useNuget.setState((s) => ({ updating: s.updating && { ...s.updating, done: s.updating.done + 1 } }));
    }
  } finally {
    useNuget.setState((s) => ({
      busy: new Set([...s.busy].filter((p) => !projects.includes(p))),
      ticked: new Set([...s.ticked].filter((id) => !updated.includes(id))),
      updating: null,
    }));
  }
  const count = updated.length;
  if (count)
    notifications().notify({
      level: "info",
      title: count === 1 ? tx`Updated 1 package` : tx`Updated ${count} packages`,
      group: "tools",
    });
  if (failed.length)
    notifications().notify({
      level: "error",
      title: tx`Some packages could not be updated`,
      body: failed.join("\n"),
      group: "tools",
    });
  await loadInstalled();
}

/** Lists the package sources; a chosen feed that is gone or disabled drops out of the choice. */
export async function loadSources(): Promise<void> {
  if (!isTauri()) return;
  const id = ++seq.sources;
  try {
    const sources = await call(commands.nugetSources());
    if (id !== seq.sources) return;
    const before = feedArgs();
    useNuget.setState((s) => {
      const enabled = sources.filter((x) => x.enabled).map((x) => x.name);
      const kept = s.feeds?.filter((f) => enabled.includes(f)) ?? [];
      // Nothing left, or every enabled source: that is all of them.
      const feeds = kept.length && kept.length < enabled.length ? kept : null;
      return { sources, sourcesError: null, feeds };
    });
    if (feedArgs().join("\n") !== before.join("\n")) feedsChanged();
  } catch (e) {
    // Sources stay unknown (null), so opening the picker or the tool window lists them again.
    if (id === seq.sources) useNuget.setState({ sourcesError: message(e) });
  }
}

/** Searches feed `name` too, or no longer; the last chosen feed stays. */
export function toggleFeed(name: string) {
  const { sources, feeds } = useNuget.getState();
  const enabled = (sources ?? []).filter((s) => s.enabled).map((s) => s.name);
  const current = feeds ?? enabled;
  const next = current.includes(name) ? current.filter((f) => f !== name) : [...current, name];
  if (!next.length) return;
  useNuget.setState({ feeds: enabled.every((e) => next.includes(e)) ? null : next });
  feedsChanged();
}

/** Searches every enabled feed. */
export function chooseAllFeeds() {
  if (useNuget.getState().feeds === null) return;
  useNuget.setState({ feeds: null });
  feedsChanged();
}

/** The search, the versions and known updates follow the chosen feeds. */
function feedsChanged() {
  const { query, selected, outdated } = useNuget.getState();
  if (query.trim()) void runSearch(false);
  if (selected) void loadVersions(selected);
  if (outdated) void checkUpdates();
}

async function sourceChange(op: () => Promise<unknown>): Promise<boolean> {
  try {
    await op();
    return true;
  } catch (e) {
    notifications().notify({ level: "error", title: tx`NuGet feed change failed`, body: message(e), group: "tools" });
    return false;
  } finally {
    await loadSources();
  }
}

export const addSource = (name: string, url: string, scope: SourceScope) =>
  sourceChange(() => call(commands.nugetAddSource(name.trim(), url.trim(), scope)));

export const updateSource = (name: string, url: string) =>
  sourceChange(() => call(commands.nugetUpdateSource(name, url.trim())));

export const removeSource = (name: string) => sourceChange(() => call(commands.nugetRemoveSource(name)));

export const enableSource = (name: string, enabled: boolean) =>
  sourceChange(() => call(commands.nugetEnableSource(name, enabled)));
