import { describe, expect, it } from "vitest";
import type { AspireResource } from "@/bindings";
import type { RunConfig } from "@/platform/ipc";
import {
  appHostConfig,
  appendLogs,
  availableCommands,
  resourceLabel,
  resourceTone,
  stateLabel,
  upsertResource,
} from "./model";

const resource = (over: Partial<AspireResource> = {}): AspireResource => ({
  name: "api-wbqs",
  displayName: "api",
  resourceType: "Project",
  state: "Running",
  health: "Healthy",
  source: "Fixture.Api.csproj",
  startTime: null,
  urls: [],
  commands: [
    { name: "start", displayName: "Start", description: null },
    { name: "stop", displayName: "Stop", description: null },
    { name: "restart", displayName: "Restart", description: null },
  ],
  pid: 1,
  projectPath: null,
  exitCode: null,
  dashboardUrl: null,
  ...over,
});

describe("aspire resources", () => {
  it("reads states as tones", () => {
    expect(resourceTone(resource())).toBe("running");
    expect(resourceTone(resource({ health: "Unhealthy" }))).toBe("warning");
    expect(resourceTone(resource({ state: "Starting" }))).toBe("starting");
    expect(resourceTone(resource({ state: "FailedToStart" }))).toBe("failed");
    expect(resourceTone(resource({ state: "Exited", exitCode: 1 }))).toBe("failed");
    expect(resourceTone(resource({ state: "Exited", exitCode: 0 }))).toBe("stopped");
    expect(stateLabel("FailedToStart")).toBe("Failed to start");
    expect(stateLabel(null)).toBe("Unknown");
  });

  it("keeps one entry per resource, in name order, and names replicas", () => {
    let list = upsertResource([], resource({ name: "web-1", displayName: "web" }));
    list = upsertResource(list, resource());
    list = upsertResource(list, resource({ state: "Exited" }));
    list = upsertResource(list, resource({ name: "web-2", displayName: "web" }));
    expect(list.map((r) => [r.name, r.state])).toEqual([
      ["api-wbqs", "Exited"],
      ["web-1", "Running"],
      ["web-2", "Running"],
    ]);
    expect(resourceLabel(list[0]!, list)).toBe("api");
    expect(resourceLabel(list[1]!, list)).toBe("web-1");
  });

  it("offers start only when stopped and stop only when up", () => {
    expect(availableCommands(resource()).map((c) => c.name)).toEqual(["stop", "restart"]);
    expect(availableCommands(resource({ state: "Exited" })).map((c) => c.name)).toEqual(["start", "restart"]);
  });

  it("caps the console", () => {
    const line = (n: number) => ({ resource: "api", content: String(n), isError: false });
    const lines = appendLogs([line(1), line(2)], [line(3), line(4)], 3);
    expect(lines.map((l) => l.content)).toEqual(["2", "3", "4"]);
  });

  it("finds the AppHost's run configuration", () => {
    const config = (id: string, project: string) =>
      ({
        id,
        command: "dotnet",
        args: ["run", "--project", project, "--launch-profile", "http"],
      }) as unknown as RunConfig;
    const configs = [config("api", "src/Api/Api.csproj"), config("host", "src/AppHost/AppHost.csproj")];
    expect(appHostConfig("/s/src/AppHost/AppHost.csproj", "/s", configs)?.id).toBe("host");
    expect(appHostConfig("/s/src/Other/Other.csproj", "/s", configs)).toBeUndefined();
  });
});
