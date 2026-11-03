/** NuGet tool window state: the search, the installed packages and the selected package. */
import { create } from "zustand";
import { commands } from "@/bindings";
import type { InstalledPackages, PackageSearch } from "@/bindings";
import { call, isTauri } from "@/platform/ipc";
import { tx } from "@/platform/i18n";
import { notifications } from "@/workbench/contributions";
import { isPrerelease } from "./model";

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
  /** Selected package id. */
  selected: string | null;
  versions: string[] | null;
  /** The version the details pane installs or updates to. */
  version: string | null;
  /** Projects an install / update / remove is running for. */
  busy: ReadonlySet<string>;
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
  selected: null,
  versions: null,
  version: null,
  busy: new Set(),
  error: null,
});

export const useNuget = create<NugetState>(initial);

export function resetNuget() {
  clearTimeout(searchTimer);
  seq.search++;
  seq.versions++;
  seq.installed++;
  useNuget.setState(initial());
}

const seq = { search: 0, versions: 0, installed: 0 };
let searchTimer: ReturnType<typeof setTimeout> | undefined;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Re-reads the packages of every project; `outdated` then asks the sources for updates. */
export async function loadInstalled(outdated = false): Promise<void> {
  if (!isTauri()) return;
  const id = ++seq.installed;
  useNuget.setState({ loading: true, error: null });
  try {
    const installed = await call(commands.nugetInstalled(false, false));
    if (id !== seq.installed) return;
    useNuget.setState({ installed, loading: false });
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
    const outdated = await call(commands.nugetInstalled(true, useNuget.getState().prerelease));
    if (id === seq.installed) useNuget.setState({ outdated, checking: false });
  } catch (e) {
    if (id === seq.installed) useNuget.setState({ checking: false, error: message(e) });
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
    const page = await call(commands.nugetSearch(query, prerelease, skip, PAGE));
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
    const versions = await call(commands.nugetVersions(pkg, useNuget.getState().prerelease));
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
