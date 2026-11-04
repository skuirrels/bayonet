import { describe, expect, it, vi } from "vitest";

const opened: string[] = [];
const notes: Array<{ level: string; title: string }> = [];
let active: string | null = null;
const PROJECTS = [
  { name: "Web", path: "/s/src/Web/Web.csproj", dir: "/s/src/Web", folders: [] },
  { name: "Web.Admin", path: "/s/src/Web/Admin/Web.Admin.csproj", dir: "/s/src/Web/Admin", folders: [] },
  { name: "Core", path: "/s/src/Core/Core.csproj", dir: "/s/src/Core", folders: [] },
];

vi.mock("@/bindings", () => ({
  commands: { dotnetUserSecrets: async (p: string) => `/home/u/.microsoft/usersecrets/${p.length}/secrets.json` },
}));
vi.mock("@/platform/ipc", () => ({ call: <T>(p: Promise<T>) => p, isTauri: () => true }));
vi.mock("@/editor/api", () => ({ activeEditor: () => (active ? { ctx: { path: active } } : null) }));
vi.mock("@/workbench/contributions", () => ({
  maybeEditorService: () => ({ openFile: async (f: string) => void opened.push(f) }),
  notifications: () => ({ notify: (n: { level: string; title: string }) => void notes.push(n) }),
}));
vi.mock("./projects/store", () => ({
  projectAt: (dir: string) => PROJECTS.find((p) => p.dir === dir) ?? null,
  useSolution: { getState: () => ({ projects: PROJECTS }) },
}));

const { manageUserSecrets, projectContaining, targetProject } = await import("./secrets");

describe("user secrets (PLT-4)", () => {
  it("picks the innermost project of a file", () => {
    expect(projectContaining(PROJECTS, "/s/src/Web/Admin/Pages/Index.cshtml")?.name).toBe("Web.Admin");
    expect(projectContaining(PROJECTS, "/s/src/Web/Program.cs")?.name).toBe("Web");
    expect(projectContaining(PROJECTS, "/s/src/WebOther/x.cs")).toBeNull();
  });

  it("uses the chosen project, else the active file's", async () => {
    expect(targetProject("/s/src/Core")?.name).toBe("Core");
    active = "/s/src/Web/Program.cs";
    expect(targetProject(null)?.name).toBe("Web");
    await manageUserSecrets(null);
    expect(opened).toEqual([`/home/u/.microsoft/usersecrets/${PROJECTS[0]!.path.length}/secrets.json`]);
  });

  it("asks for a project when it cannot tell which", async () => {
    active = "/s/README.md";
    await manageUserSecrets(null);
    expect(notes.at(-1)?.level).toBe("info");
  });
});
