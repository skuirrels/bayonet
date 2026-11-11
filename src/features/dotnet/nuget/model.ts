/** NuGet tool window model: installed packages across projects, versions, search rows. */
import type { InstalledPackages, PackageSearch, PackageSource, PackageSummary } from "@/bindings";

/** One project's reference to a package, over all its target frameworks. */
export interface ProjectUse {
  /** Project file path. */
  project: string;
  /** `Fixture.Api` from `…/Fixture.Api.csproj`. */
  name: string;
  /** Resolved version, else the requested one. */
  version: string;
  latest: string | null;
  autoReferenced: boolean;
  frameworks: string[];
}

export interface InstalledPackage {
  id: string;
  uses: ProjectUse[];
  /** Newest version in use, for the list. */
  version: string;
  /** The newest version on the sources when it is newer than one in use. */
  update: string | null;
  /** Only implicit SDK references: shown, never removable. */
  implicit: boolean;
}

export interface ProjectRef {
  path: string;
  name: string;
  /** `Directory.Packages.props` holding its versions (central package management). */
  central: string | null;
}

/** `Fixture.Api` from a project file path. */
export function projectName(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  return file.replace(/\.[^.]+proj$/i, "");
}

/** NuGet version order: numbers, then a release after its prereleases; build metadata ignored. */
export function compareVersions(a: string, b: string): number {
  const split = (v: string) => {
    const core = v.split("+")[0]!;
    const dash = core.indexOf("-");
    const nums = (dash < 0 ? core : core.slice(0, dash)).split(".").map((n) => Number.parseInt(n, 10) || 0);
    while (nums.length < 4) nums.push(0);
    return { nums, pre: dash < 0 ? null : core.slice(dash + 1) };
  };
  const x = split(a);
  const y = split(b);
  for (let i = 0; i < Math.max(x.nums.length, y.nums.length); i++) {
    const d = (x.nums[i] ?? 0) - (y.nums[i] ?? 0);
    if (d) return Math.sign(d);
  }
  if (x.pre === null || y.pre === null) return x.pre === y.pre ? 0 : x.pre === null ? 1 : -1;
  const xs = x.pre.split(".");
  const ys = y.pre.split(".");
  for (let i = 0; i < Math.max(xs.length, ys.length); i++) {
    const p = xs[i];
    const q = ys[i];
    if (p === undefined) return -1;
    if (q === undefined) return 1;
    const pn = /^\d+$/.test(p);
    const qn = /^\d+$/.test(q);
    const d = pn && qn ? Number(p) - Number(q) : pn ? -1 : qn ? 1 : p.toLowerCase().localeCompare(q.toLowerCase());
    if (d) return Math.sign(d);
  }
  return 0;
}

export const isPrerelease = (v: string) => v.split("+")[0]!.includes("-");

/** The projects of a listing, by name. */
export function projectsOf(list: InstalledPackages | null): ProjectRef[] {
  return (list?.projects ?? [])
    .map((p) => ({
      path: p.path,
      name: projectName(p.path),
      central: p.centralPackages,
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Packages in use, by id: each with the projects that reference it. `outdated`, a
 * listing with `--outdated`, adds the latest versions.
 */
export function installedPackages(
  list: InstalledPackages | null,
  outdated?: InstalledPackages | null,
): InstalledPackage[] {
  const latest = new Map<string, string>();
  for (const p of outdated?.projects ?? [])
    for (const f of p.frameworks)
      for (const x of f.packages) if (x.latest) latest.set(`${p.path}\n${x.id.toLowerCase()}`, x.latest);
  const byId = new Map<string, InstalledPackage>();
  for (const p of list?.projects ?? []) {
    for (const f of p.frameworks) {
      for (const x of f.packages) {
        const key = x.id.toLowerCase();
        let pkg = byId.get(key);
        if (!pkg) {
          pkg = {
            id: x.id,
            uses: [],
            version: "",
            update: null,
            implicit: true,
          };
          byId.set(key, pkg);
        }
        let use = pkg.uses.find((u) => u.project === p.path);
        const version = x.resolved ?? x.requested ?? "";
        if (!use) {
          use = {
            project: p.path,
            name: projectName(p.path),
            version,
            latest: x.latest ?? latest.get(`${p.path}\n${key}`) ?? null,
            autoReferenced: x.autoReferenced,
            frameworks: [],
          };
          pkg.uses.push(use);
        } else if (version && compareVersions(version, use.version) > 0) {
          use.version = version;
        }
        use.frameworks.push(f.framework);
        if (!x.autoReferenced) pkg.implicit = false;
      }
    }
  }
  const out = [...byId.values()];
  for (const pkg of out) {
    pkg.uses.sort((a, b) => a.name.localeCompare(b.name));
    pkg.version = pkg.uses.map((u) => u.version).reduce((a, b) => (compareVersions(a, b) >= 0 ? a : b), "");
    const newest = pkg.uses
      .map((u) => u.latest)
      .filter((v): v is string => !!v)
      .reduce<string | null>((a, b) => (a === null || compareVersions(b, a) > 0 ? b : a), null);
    pkg.update = newest && pkg.uses.some((u) => compareVersions(newest, u.version) > 0) ? newest : null;
  }
  return out.sort((a, b) => Number(a.implicit) - Number(b.implicit) || a.id.localeCompare(b.id));
}

/** Search results, once per id (the first source that has it wins). */
export function searchRows(search: PackageSearch | null): Array<PackageSummary & { source: string }> {
  const seen = new Set<string>();
  const rows: Array<PackageSummary & { source: string }> = [];
  for (const s of search?.sources ?? []) {
    for (const p of s.packages) {
      const key = p.id.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({ ...p, source: s.source });
    }
  }
  return rows;
}

/** Problems of a search, prefixed by their source. */
export function searchProblems(search: PackageSearch | null): string[] {
  return [
    ...(search?.problems ?? []),
    ...(search?.sources ?? []).flatMap((s) => s.problems.map((p) => (s.source ? `${s.source}: ${p}` : p))),
  ];
}

/** `173894242` → `173.9M`. */
export function formatDownloads(n: number | null | undefined): string {
  if (n == null) return "";
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(n);
}

/** What the details pane offers for one project and the chosen version. */
export type ProjectAction = "install" | "update" | "downgrade" | "none";

export function projectAction(use: ProjectUse | undefined, version: string | null): ProjectAction {
  if (!use) return "install";
  if (!version || use.autoReferenced) return "none";
  const d = compareVersions(version, use.version);
  return d > 0 ? "update" : d < 0 ? "downgrade" : "none";
}

/** One package's move to its newest version: the projects that use an older one. */
export interface PackageUpdate {
  id: string;
  version: string;
  projects: string[];
}

/** What Update All / Update Selected runs for `packages`: those with an update, in projects that can take it. */
export function updatesOf(packages: readonly InstalledPackage[]): PackageUpdate[] {
  const out: PackageUpdate[] = [];
  for (const p of packages) {
    const version = p.update;
    if (!version) continue;
    const projects = p.uses
      .filter((u) => !u.autoReferenced && compareVersions(version, u.version) > 0)
      .map((u) => u.project);
    if (projects.length) out.push({ id: p.id, version, projects });
  }
  return out;
}

/**
 * The `--source` URLs for the feeds chosen by name: none (every enabled source) when
 * nothing is chosen or every enabled source is.
 */
export function chosenSources(sources: readonly PackageSource[] | null, feeds: readonly string[] | null): string[] {
  if (!sources || !feeds) return [];
  const enabled = sources.filter((s) => s.enabled);
  const chosen = enabled.filter((s) => feeds.includes(s.name));
  return chosen.length === 0 || chosen.length === enabled.length ? [] : chosen.map((s) => s.url);
}
