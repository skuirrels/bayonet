import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "zustand";
import type { GeneratedSources } from "@/bindings";

const sources = vi.fn<(project: string) => Promise<GeneratedSources>>();
const notify = vi.fn();
const tree = create<{ expanded: ReadonlySet<string> }>(() => ({ expanded: new Set() }));

vi.mock("@/bindings", () => ({
  commands: {
    dotnetGeneratedDirs: async (projects: string[]) => projects.map((p) => `/cache/${p.split("/").pop()}`),
    dotnetGeneratedSources: (project: string) => sources(project),
  },
}));
vi.mock("@/platform/ipc", () => ({ call: <T>(p: Promise<T>) => p }));
vi.mock("@/platform/actions", () => ({ executeAction: vi.fn(async () => true) }));
vi.mock("@/workbench/contributions", () => ({ notifications: () => ({ notify }) }));
vi.mock("@/features/project-tree/store", () => ({
  reloadLoadedDirsUnder: vi.fn(async () => {}),
  useProjectTree: tree,
}));

const { loadGeneratedDirs, resetGenerated, sourcesChanged, useGenerated, watchGenerated } = await import("./generated");

const API = "/s/src/Api/Api.csproj";
const result = (over: Partial<GeneratedSources> = {}): GeneratedSources => ({
  dir: "/cache/Api.csproj",
  analyzers: ["Gen"],
  files: ["/cache/Api.csproj/Gen/Gen.Names/Names.g.cs"],
  errors: [],
  stale: false,
  needsBuild: false,
  ...over,
});
const expand = (...paths: string[]) => tree.setState({ expanded: new Set(paths) });
const flush = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

let unwatch = () => {};

beforeEach(async () => {
  vi.useFakeTimers();
  sources.mockReset();
  notify.mockReset();
  resetGenerated();
  tree.setState({ expanded: new Set() });
  await loadGeneratedDirs([API, "/s/src/Lib/Lib.fsproj"]);
  unwatch = watchGenerated();
});

afterEach(() => {
  unwatch();
  vi.useRealTimers();
});

describe("generated sources", () => {
  it("knows cache folders for C# and VB projects only", () => {
    expect(useGenerated.getState().dirs).toEqual({ [API]: "/cache/Api.csproj" });
  });

  it("compiles when the Analyzers node is first expanded, not again on re-expand", async () => {
    sources.mockResolvedValue(result());
    expand("/cache/Api.csproj");
    await flush();
    expect(sources).toHaveBeenCalledTimes(1);
    expect(useGenerated.getState().results[API]?.files).toHaveLength(1);
    expand();
    expand("/cache/Api.csproj");
    await flush();
    expect(sources).toHaveBeenCalledTimes(1);
  });

  it("recompiles shortly after a source change, ignoring build output", async () => {
    sources.mockResolvedValue(result());
    expand("/cache/Api.csproj");
    await flush();
    sourcesChanged(["/s/src/Api/obj/Debug/net10.0/Api.AssemblyInfo.cs", "/s/src/Api/bin/Debug/Api.deps.json"]);
    await vi.advanceTimersByTimeAsync(2000);
    expect(sources).toHaveBeenCalledTimes(1);
    sourcesChanged(["/s/src/Api/Program.cs"]);
    await vi.advanceTimersByTimeAsync(2000);
    expect(sources).toHaveBeenCalledTimes(2);
  });

  it("runs again after a compile when the sources changed during it", async () => {
    let finish = (_: GeneratedSources) => {};
    sources.mockImplementationOnce(() => new Promise((r) => (finish = r))).mockResolvedValue(result());
    expand("/cache/Api.csproj");
    await flush();
    expect(useGenerated.getState().running.has(API)).toBe(true);
    sourcesChanged(["/s/src/Api/Program.cs"]);
    await vi.advanceTimersByTimeAsync(2000);
    // Still running: the timer's call is absorbed, then the change reruns it.
    expect(sources).toHaveBeenCalledTimes(1);
    finish(result());
    await flush();
    await flush();
    expect(sources).toHaveBeenCalledTimes(2);
  });

  it("does not compile collapsed projects, but does on the next expand", async () => {
    sources.mockResolvedValue(result());
    expand("/cache/Api.csproj");
    await flush();
    expand();
    sourcesChanged(["/s/Directory.Build.props"]);
    await vi.advanceTimersByTimeAsync(2000);
    expect(sources).toHaveBeenCalledTimes(1);
    expand("/cache/Api.csproj");
    await flush();
    expect(sources).toHaveBeenCalledTimes(2);
  });

  it("offers a build when referenced projects are not built", async () => {
    sources.mockResolvedValue(result({ files: [], needsBuild: true, errors: ["error CS0006: Metadata file"] }));
    expand("/cache/Api.csproj");
    await flush();
    expect(notify).toHaveBeenCalledWith(expect.objectContaining({ key: `generated-needs-build-${API}` }));
  });
});
