import { describe, expect, it } from "vitest";
import type { FrameworkDependencies, ProjectDependencies } from "@/bindings";
import { dependencyEntries, touchesDependencies } from "./dependencies";
import { solutionRoot } from "./solution";

const fw = (over: Partial<FrameworkDependencies> = {}): FrameworkDependencies => ({
  framework: "net10.0",
  frameworkReferences: ["Microsoft.NETCore.App", "Microsoft.AspNetCore.App"],
  packages: [
    { id: "Serilog.AspNetCore", version: "9.0.0", topLevel: true, dependencies: ["Serilog"] },
    { id: "Serilog", version: "4.2.0", topLevel: false, dependencies: [] },
  ],
  projects: ["/w/src/Core/Core.csproj", "/w/src/Lib/Lib.fsproj"],
  ...over,
});

const labels = (entries: readonly { label: string }[]) => entries.map((e) => e.label);

describe("dependencyEntries", () => {
  it("lists frameworks, packages with what they bring in, and project references", () => {
    const [frameworks, packages, projects] = dependencyEntries({ restored: true, frameworks: [fw()] });
    expect(frameworks).toMatchObject({ label: "Frameworks", icon: "library" });
    expect(labels(frameworks!.children!)).toEqual(["Microsoft.NETCore.App", "Microsoft.AspNetCore.App"]);
    expect(frameworks!.children!.every((c) => c.children?.length === 0)).toBe(true);

    expect(packages).toMatchObject({ label: "Packages", icon: "toolwindow-nuget", detail: undefined });
    // Only top-level packages at the first level; transitive ones below them.
    expect(packages!.children).toEqual([
      {
        id: "Serilog.AspNetCore",
        label: "Serilog.AspNetCore",
        icon: "package",
        detail: "9.0.0",
        children: [{ id: "Serilog", label: "Serilog", icon: "package", detail: "4.2.0", children: [] }],
      },
    ]);

    expect(projects).toMatchObject({ label: "Projects", icon: "module" });
    expect(projects!.children).toEqual([
      { id: "/w/src/Core/Core.csproj", label: "Core", icon: "project-csharp", reveal: "/w/src/Core", children: [] },
      { id: "/w/src/Lib/Lib.fsproj", label: "Lib", icon: "project-fsharp", reveal: "/w/src/Lib", children: [] },
    ]);
  });

  it("leaves out empty groups and marks packages read from the project file", () => {
    const deps: ProjectDependencies = {
      restored: false,
      frameworks: [fw({ projects: [], packages: [{ id: "Dapper", version: null, topLevel: true, dependencies: [] }] })],
    };
    const entries = dependencyEntries(deps);
    expect(labels(entries)).toEqual(["Frameworks", "Packages"]);
    expect(entries[1]).toMatchObject({ detail: "not restored" });
    expect(entries[1]!.children![0]).toMatchObject({ label: "Dapper", detail: undefined });
  });

  it("groups a multi-targeted project's dependencies by target framework", () => {
    const deps: ProjectDependencies = {
      restored: true,
      frameworks: [fw(), fw({ framework: "net48", frameworkReferences: [], projects: [] })],
    };
    const entries = dependencyEntries(deps);
    expect(labels(entries)).toEqual(["net10.0", "net48"]);
    expect(labels(entries[0]!.children!)).toEqual(["Frameworks", "Packages", "Projects"]);
    expect(labels(entries[1]!.children!)).toEqual(["Packages"]);
  });

  it("stops at a dependency cycle", () => {
    const deps: ProjectDependencies = {
      restored: true,
      frameworks: [
        fw({
          frameworkReferences: [],
          projects: [],
          packages: [
            { id: "A", version: "1.0.0", topLevel: true, dependencies: ["B"] },
            { id: "B", version: "1.0.0", topLevel: false, dependencies: ["A"] },
          ],
        }),
      ],
    };
    const [packages] = dependencyEntries(deps);
    const a = packages!.children![0]!;
    expect(a.children![0]).toMatchObject({ label: "B", children: [] });
  });

  it("has nothing for a project without dependencies", () => {
    expect(dependencyEntries(undefined)).toEqual([]);
    expect(dependencyEntries({ restored: true, frameworks: [] })).toEqual([]);
  });
});

describe("solution tree dependencies", () => {
  it("puts Analyzers first, then the project's dependencies", () => {
    const root = solutionRoot(
      {
        path: "/w/App.slnx",
        name: "App",
        format: "slnx",
        items: [{ kind: "project", name: "App", path: "/w/App/App.csproj", language: "csharp", missing: false }],
      },
      { "/w/App/App.csproj": "/cache/App" },
      { "/w/App/App.csproj": { restored: true, frameworks: [fw()] } },
    );
    const [deps] = root.children![0]!.prepend!;
    expect(deps).toMatchObject({ id: "dependencies", label: "Dependencies" });
    expect(labels(deps!.children!)).toEqual(["Analyzers", "Frameworks", "Packages", "Projects"]);
  });

  it("shows Dependencies for an F# project, which has no Analyzers node", () => {
    const root = solutionRoot(
      {
        path: "/w/App.slnx",
        name: "App",
        format: "slnx",
        items: [{ kind: "project", name: "Lib", path: "/w/Lib/Lib.fsproj", language: "fsharp", missing: false }],
      },
      {},
      { "/w/Lib/Lib.fsproj": { restored: true, frameworks: [fw()] } },
    );
    expect(labels(root.children![0]!.prepend![0]!.children!)).toEqual(["Frameworks", "Packages", "Projects"]);
  });
});

describe("touchesDependencies", () => {
  it("matches restore output only", () => {
    expect(touchesDependencies(["/w/App/obj/project.assets.json"])).toBe(true);
    expect(touchesDependencies(["/w/App/obj/App.csproj.nuget.g.props", "/w/App/Program.cs"])).toBe(false);
  });
});
