import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AspireEvent, AspireLogLine, AspireResource } from "@/bindings";

type Sink<T> = { onmessage: (m: T) => void };
let watchSink: Sink<AspireEvent> | null = null;
let logSinks: Array<{ resource: string; sink: Sink<AspireLogLine[]> }> = [];
const cancelled: number[] = [];
const notify = vi.fn();
const aspireCommand = vi.fn(async (_a: string, _r: string, _n: string) => "ok");
const executeAction = vi.fn(async (_id: string, _arg?: unknown) => true);
let nextId = 0;

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (m: unknown) => void = () => {};
  },
}));
vi.mock("@/bindings", () => ({
  commands: {
    aspireCli: async () => ({ path: "/bin/aspire", install: { command: "dotnet", args: [], cwd: null } }),
    aspireApphosts: async (projects: string[]) => projects.filter((p) => p.includes("AppHost")),
    aspireWatch: (_r: number, _a: string, channel: Sink<AspireEvent>) => {
      watchSink = channel;
      return new Promise(() => {});
    },
    aspireLogs: (_r: number, _a: string, resource: string, channel: Sink<AspireLogLine[]>) => {
      logSinks.push({ resource, sink: channel });
      return new Promise(() => {});
    },
    aspireCommand: (a: string, r: string, n: string) => aspireCommand(a, r, n),
    cancelRequest: async (r: number) => void cancelled.push(r),
  },
}));
vi.mock("@/platform/ipc", () => ({
  call: <T>(p: Promise<T>) => p,
  isCancelled: () => false,
  isTauri: () => true,
  nextRequestId: () => ++nextId,
}));
vi.mock("@/platform/actions", () => ({ executeAction: (id: string, arg?: unknown) => executeAction(id, arg) }));
vi.mock("@/platform/openUrl", () => ({ openExternalUrl: vi.fn() }));
vi.mock("@/platform/project", () => ({ useProject: { getState: () => ({ project: { root: "/s" } }) } }));
vi.mock("@/workbench/contributions", () => ({ notifications: () => ({ notify }) }));
vi.mock("@/features/run/store", () => ({
  loadConfigs: async () => {},
  useRun: {
    getState: () => ({
      configs: [
        {
          id: "dotnet:run:AppHost:http",
          command: "dotnet",
          args: ["run", "--project", "src/AppHost/AppHost.csproj", "--launch-profile", "http"],
        },
      ],
    }),
  },
}));

const store = await import("./store");
const { useAspire } = store;

const APPHOST = "/s/src/AppHost/AppHost.csproj";
const resource = (over: Partial<AspireResource> = {}): AspireResource => ({
  name: "api-x1",
  displayName: "api",
  resourceType: "Project",
  state: "Running",
  health: "Healthy",
  source: null,
  startTime: null,
  urls: [],
  commands: [{ name: "stop", displayName: "Stop", description: null }],
  pid: null,
  projectPath: null,
  exitCode: null,
  dashboardUrl: null,
  ...over,
});
const flush = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};
const event = (e: AspireEvent) => watchSink!.onmessage(e);

beforeEach(async () => {
  store.resetAspire();
  watchSink = null;
  logSinks = [];
  cancelled.length = 0;
  notify.mockReset();
  aspireCommand.mockClear();
  executeAction.mockClear();
  await store.loadAppHosts(["/s/src/Api/Api.csproj", APPHOST]);
  void store.startWatch();
  await flush();
});

describe("aspire store", () => {
  it("finds the AppHost and watches it", () => {
    expect(useAspire.getState().apphosts).toEqual([APPHOST]);
    expect(useAspire.getState().selected).toBe(APPHOST);
    expect(watchSink).not.toBeNull();
  });

  it("lists resources while the AppHost runs and clears them when it stops", () => {
    event({ kind: "appHost", running: { path: APPHOST, pid: 1, dashboardUrl: "http://localhost:15178/login?t=1" } });
    event({ kind: "resource", resource: resource() });
    event({
      kind: "resource",
      resource: resource({ name: "cache-y2", displayName: "cache", resourceType: "Container" }),
    });
    event({ kind: "resource", resource: resource({ state: "Starting" }) });
    const s = useAspire.getState();
    expect(s.known).toBe(true);
    expect(s.resources.map((r) => [r.displayName, r.state])).toEqual([
      ["api", "Starting"],
      ["cache", "Running"],
    ]);
    event({ kind: "appHost", running: null });
    expect(useAspire.getState().resources).toEqual([]);
  });

  it("streams the selected resource's console only while the AppHost runs", async () => {
    store.selectResource("api-x1");
    await flush();
    expect(logSinks).toHaveLength(0);
    event({ kind: "appHost", running: { path: APPHOST, pid: 1, dashboardUrl: null } });
    await flush();
    expect(logSinks.map((l) => l.resource)).toEqual(["api-x1"]);
    logSinks[0]!.sink.onmessage([{ resource: "api", content: "started", isError: false }]);
    expect(useAspire.getState().logs).toHaveLength(1);
    store.selectResource("cache-y2");
    await flush();
    expect(cancelled.length).toBe(1);
    expect(useAspire.getState().logs).toEqual([]);
    // Lines from the stream just cancelled are dropped.
    logSinks[0]!.sink.onmessage([{ resource: "api", content: "late", isError: false }]);
    expect(useAspire.getState().logs).toEqual([]);
  });

  it("starts the AppHost through its run configuration", async () => {
    event({ kind: "appHost", running: null });
    await store.runAppHost();
    expect(executeAction).toHaveBeenCalledWith("run.runConfig", "dotnet:run:AppHost:http");
    expect(useAspire.getState().starting).toBe(true);
    event({ kind: "appHost", running: { path: APPHOST, pid: 1, dashboardUrl: null } });
    expect(useAspire.getState().starting).toBe(false);
  });

  it("reports a failed resource command", async () => {
    aspireCommand.mockRejectedValueOnce(new Error("resource not found"));
    await store.runCommand(resource(), "restart");
    expect(aspireCommand).toHaveBeenCalledWith(APPHOST, "api-x1", "restart");
    expect(notify).toHaveBeenCalledWith(expect.objectContaining({ level: "error", body: "resource not found" }));
    expect(useAspire.getState().busy.size).toBe(0);
  });
});
