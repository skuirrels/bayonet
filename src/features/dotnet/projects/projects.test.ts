import { describe, expect, it } from "vitest";
import type { ProjectTemplate, SolutionItem } from "@/bindings";
import { matchesTemplate, sortTemplates, validName } from "./NewProjectDialog";
import { referenceChanges } from "./ReferencesDialog";
import { projectsOf } from "./store";

const template = (shortName: string, name: string, tags: string[] = []): ProjectTemplate => ({
  id: shortName,
  name,
  shortName,
  description: null,
  author: null,
  tags,
  languages: ["C#"],
  kind: "project",
  parameters: [],
});

describe("solution edits", () => {
  it("lists the solution's projects with their folders", () => {
    const items: SolutionItem[] = [
      {
        kind: "folder",
        name: "src",
        items: [
          {
            kind: "project",
            name: "App",
            path: "/s/src/App/App.csproj",
            language: "csharp",
            missing: false,
          },
        ],
      },
      {
        kind: "project",
        name: "Gone",
        path: "/s/Gone/Gone.csproj",
        language: "csharp",
        missing: true,
      },
      { kind: "file", path: "/s/README.md" },
    ];
    expect(projectsOf(items)).toEqual([
      {
        name: "App",
        path: "/s/src/App/App.csproj",
        dir: "/s/src/App",
        folders: ["src"],
      },
    ]);
  });

  it("orders common templates first and searches names, short names and tags", () => {
    const list = [
      template("zeta", "Zeta"),
      template("webapi", "ASP.NET Core Web API", ["Web"]),
      template("console", "Console App"),
    ];
    expect(sortTemplates(list).map((t) => t.shortName)).toEqual(["console", "webapi", "zeta"]);
    expect(matchesTemplate(list[1]!, "web")).toBe(true);
    expect(matchesTemplate(list[1]!, "WEBAPI")).toBe(true);
    expect(matchesTemplate(list[0]!, "web")).toBe(false);
  });

  it("accepts project names that make valid file names", () => {
    expect(validName("Acme.Core")).toBe(true);
    expect(validName("my-app")).toBe(true);
    expect(validName("1st")).toBe(false);
    expect(validName("a/b")).toBe(false);
  });

  it("turns ticked references into additions and removals", () => {
    expect(referenceChanges(new Set(["a", "b"]), new Set(["b", "c"]))).toEqual({
      add: ["c"],
      remove: ["a"],
    });
  });
});
