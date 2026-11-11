import { describe, expect, it } from "vitest";
import type { InstalledPackages, PackageSearch, PackageSource } from "@/bindings";
import {
  chosenSources,
  compareVersions,
  formatDownloads,
  installedPackages,
  projectAction,
  projectName,
  projectsOf,
  searchProblems,
  searchRows,
  updatesOf,
  type InstalledPackage,
} from "./model";

const API = "/s/src/Fixture.Api/Fixture.Api.csproj";
const CORE = "/s/src/Fixture.Core/Fixture.Core.csproj";

const list: InstalledPackages = {
  problems: [],
  projects: [
    {
      path: API,
      centralPackages: "/s/Directory.Packages.props",
      frameworks: [
        {
          framework: "net10.0",
          packages: [
            {
              id: "Humanizer",
              requested: "2.14.1",
              resolved: "2.14.1",
              latest: null,
              autoReferenced: false,
            },
            {
              id: "Microsoft.AspNetCore.OpenApi",
              requested: "10.0.14",
              resolved: "10.0.14",
              latest: null,
              autoReferenced: false,
            },
          ],
        },
      ],
    },
    {
      path: CORE,
      centralPackages: null,
      frameworks: [
        {
          framework: "netstandard2.0",
          packages: [
            {
              id: "NETStandard.Library",
              requested: "[2.0.3, )",
              resolved: "2.0.3",
              latest: null,
              autoReferenced: true,
            },
            {
              id: "humanizer",
              requested: "3.0.10",
              resolved: "3.0.10",
              latest: null,
              autoReferenced: false,
            },
          ],
        },
        {
          framework: "net10.0",
          packages: [
            {
              id: "Humanizer",
              requested: "3.0.10",
              resolved: "3.0.10",
              latest: null,
              autoReferenced: false,
            },
          ],
        },
      ],
    },
  ],
};

describe("nuget model", () => {
  it("orders versions like NuGet", () => {
    expect(compareVersions("1.10.0", "1.9.0")).toBe(1);
    expect(compareVersions("1.0", "1.0.0.0")).toBe(0);
    expect(compareVersions("1.0.0-beta", "1.0.0")).toBe(-1);
    expect(compareVersions("1.0.0-beta.10", "1.0.0-beta.9")).toBe(1);
    expect(compareVersions("1.0.0-alpha", "1.0.0-alpha.1")).toBe(-1);
    expect(compareVersions("1.0.0+build", "1.0.0")).toBe(0);
  });

  it("groups references by package id across projects and frameworks", () => {
    const pkgs = installedPackages(list);
    expect(pkgs.map((p) => p.id)).toEqual(["Humanizer", "Microsoft.AspNetCore.OpenApi", "NETStandard.Library"]);
    const h = pkgs[0]!;
    expect(h.version).toBe("3.0.10");
    expect(h.uses.map((u) => [u.name, u.version, u.frameworks])).toEqual([
      ["Fixture.Api", "2.14.1", ["net10.0"]],
      ["Fixture.Core", "3.0.10", ["netstandard2.0", "net10.0"]],
    ]);
    expect(h.update).toBeNull();
    expect(pkgs[2]!.implicit).toBe(true);
  });

  it("takes updates from the outdated listing", () => {
    const outdated: InstalledPackages = {
      problems: [],
      projects: [
        {
          path: API,
          centralPackages: null,
          frameworks: [
            {
              framework: "net10.0",
              packages: [
                {
                  id: "Humanizer",
                  requested: "2.14.1",
                  resolved: "2.14.1",
                  latest: "3.0.10",
                  autoReferenced: false,
                },
              ],
            },
          ],
        },
      ],
    };
    const h = installedPackages(list, outdated)[0]!;
    expect(h.update).toBe("3.0.10");
    expect(h.uses[0]!.latest).toBe("3.0.10");
  });

  it("names projects and lists them with their central package file", () => {
    expect(projectName("C:\\s\\A.B.fsproj")).toBe("A.B");
    expect(projectsOf(list).map((p) => [p.name, p.central])).toEqual([
      ["Fixture.Api", "/s/Directory.Packages.props"],
      ["Fixture.Core", null],
    ]);
  });

  it("offers install, update, downgrade or nothing per project", () => {
    const [h] = installedPackages(list);
    const [api, core] = h!.uses;
    expect(projectAction(undefined, "3.0.10")).toBe("install");
    expect(projectAction(api, "3.0.10")).toBe("update");
    expect(projectAction(core, "2.14.1")).toBe("downgrade");
    expect(projectAction(core, "3.0.10")).toBe("none");
    expect(projectAction({ ...core!, autoReferenced: true }, "4.0.0")).toBe("none");
  });

  it("lists search results once per id with source problems", () => {
    const search: PackageSearch = {
      problems: ["bad config"],
      sources: [
        {
          source: "nuget.org",
          problems: [],
          packages: [
            {
              id: "Humanizer",
              version: "3.0.10",
              description: null,
              owners: null,
              totalDownloads: 5,
              projectUrl: null,
            },
          ],
        },
        {
          source: "private",
          problems: ["401 Unauthorized"],
          packages: [
            {
              id: "humanizer",
              version: "3.0.9",
              description: null,
              owners: null,
              totalDownloads: null,
              projectUrl: null,
            },
            {
              id: "Acme.Core",
              version: "1.0.0",
              description: null,
              owners: null,
              totalDownloads: null,
              projectUrl: null,
            },
          ],
        },
      ],
    };
    expect(searchRows(search).map((r) => [r.id, r.source])).toEqual([
      ["Humanizer", "nuget.org"],
      ["Acme.Core", "private"],
    ]);
    expect(searchProblems(search)).toEqual(["bad config", "private: 401 Unauthorized"]);
  });

  it("formats download counts", () => {
    expect(formatDownloads(173894242)).toBe("173.9M");
    expect(formatDownloads(1196514395)).toBe("1.2B");
    expect(formatDownloads(950)).toBe("950");
    expect(formatDownloads(null)).toBe("");
  });
});

describe("updatesOf", () => {
  const use = (project: string, version: string, autoReferenced = false) => ({
    project,
    name: project,
    version,
    latest: null,
    autoReferenced,
    frameworks: ["net10.0"],
  });
  const pkg = (id: string, update: string | null, uses: InstalledPackage["uses"]): InstalledPackage => ({
    id,
    uses,
    version: uses[0]?.version ?? "",
    update,
    implicit: false,
  });

  it("moves only the projects behind the newest version", () => {
    expect(updatesOf([pkg("Humanizer", "3.0.10", [use(API, "2.14.1"), use(CORE, "3.0.10")])])).toEqual([
      { id: "Humanizer", version: "3.0.10", projects: [API] },
    ]);
  });

  it("leaves out packages with no update and SDK references", () => {
    expect(
      updatesOf([
        pkg("Newtonsoft.Json", null, [use(API, "13.0.3")]),
        pkg("NETStandard.Library", "2.0.4", [use(CORE, "2.0.3", true)]),
      ]),
    ).toEqual([]);
  });
});

describe("chosenSources", () => {
  const sources: PackageSource[] = [
    { name: "nuget.org", url: "https://api.nuget.org/v3/index.json", enabled: true },
    { name: "Team", url: "https://team/v3/index.json", enabled: true },
    { name: "Old", url: "https://old/v3/index.json", enabled: false },
  ];

  it("searches every enabled source when none or all are chosen", () => {
    expect(chosenSources(sources, null)).toEqual([]);
    expect(chosenSources(sources, ["nuget.org", "Team"])).toEqual([]);
    expect(chosenSources(null, ["Team"])).toEqual([]);
  });

  it("passes the chosen enabled sources by URL", () => {
    expect(chosenSources(sources, ["Team"])).toEqual(["https://team/v3/index.json"]);
    // A disabled source cannot be searched, so choosing only it searches them all.
    expect(chosenSources(sources, ["Old"])).toEqual([]);
  });
});
