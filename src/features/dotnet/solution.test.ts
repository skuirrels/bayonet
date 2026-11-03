import { describe, expect, it } from "vitest";
import type { Solution } from "@/bindings";
import { solutionRoot, touchesSolution } from "./solution";

const SOLUTION: Solution = {
  path: "/w/App.slnx",
  name: "App",
  format: "slnx",
  items: [
    {
      kind: "folder",
      name: "src",
      items: [
        {
          kind: "project",
          name: "App",
          path: "/w/src/App/App.csproj",
          language: "csharp",
          missing: false,
        },
        {
          kind: "project",
          name: "Gone",
          path: "/w/src/Gone/Gone.fsproj",
          language: "fsharp",
          missing: true,
        },
      ],
    },
    {
      kind: "project",
      name: "Tool",
      path: "/w/Tool/Tool.vbproj",
      language: "visualbasic",
      missing: false,
    },
    { kind: "file", path: "/w/README.md" },
  ],
};

describe("solutionRoot", () => {
  it("shows folders as groups, projects as their folders and items as files", () => {
    const root = solutionRoot(SOLUTION);
    expect(root).toMatchObject({
      id: "solution",
      label: "Solution 'App'",
      detail: "3 projects",
      scope: "solution",
    });
    expect(root.children).toEqual([
      {
        label: "src",
        icon: "folder",
        children: [
          {
            label: "App",
            icon: "file-csharp",
            path: "/w/src/App",
            detail: undefined,
            hide: BUILD_OUTPUT,
          },
          {
            label: "Gone",
            icon: "file-fsharp",
            path: "/w/src/Gone",
            detail: "not found",
            hide: BUILD_OUTPUT,
          },
        ],
      },
      {
        label: "Tool",
        icon: "module",
        path: "/w/Tool",
        detail: undefined,
        hide: BUILD_OUTPUT,
      },
      { label: "README.md", icon: "", path: "/w/README.md", file: true },
    ]);
  });
});

const BUILD_OUTPUT = ["bin", "obj"];

describe("touchesSolution", () => {
  it("matches solution and project files only", () => {
    expect(touchesSolution(["/w/App.SLN"])).toBe(true);
    expect(touchesSolution(["/w/src/App/App.csproj"])).toBe(true);
    expect(touchesSolution(["/w/src/App/Program.cs", "/w/global.json"])).toBe(false);
  });
});
