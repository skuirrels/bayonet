import { describe, expect, it, vi } from "vitest";
import type { PublishProfile, PublishTarget, RunConfig } from "@/bindings";

const choices: string[] = [];
const runs: RunConfig[] = [];
const targets: PublishTarget[] = [];
let pick = "";
let answer: string | null = null;

vi.mock("@/bindings", () => ({
  commands: {
    dotnetPublishProfiles: async (): Promise<PublishProfile[]> => [
      {
        name: "FolderProfile",
        path: "/s/Web/Properties/PublishProfiles/FolderProfile.pubxml",
        method: null,
        target: "bin/publish",
        configuration: "Release",
        runtime: null,
      },
    ],
    dotnetContainerRun: async () => ({
      command: "dotnet publish x && docker run --rm -i shop-web:dev",
      args: [],
      cwd: "/s/Web",
    }),
    dotnetPublishCommand: async (_p: string, target: PublishTarget) => {
      targets.push(target);
      return { command: "/usr/local/share/dotnet/dotnet", args: ["publish", "/s/Web/Web.csproj"], cwd: "/s/Web" };
    },
  },
}));
vi.mock("@/platform/ipc", () => ({ call: <T>(p: Promise<T>) => p, isTauri: () => true }));
vi.mock("@/platform/actions", () => ({
  executeAction: async (_id: string, config: RunConfig) => void runs.push(config),
}));
vi.mock("@/workbench/inputDialogs", () => ({
  chooseItem: async (o: { items: string[] }) => {
    choices.splice(0, choices.length, ...o.items);
    return o.items.find((i) => i.startsWith(pick)) ?? null;
  },
  promptInput: async () => answer,
}));
vi.mock("@/workbench/contributions", () => ({ notifications: () => ({ notify: () => {} }) }));

const { publishProject, repositoryName, runInContainer } = await import("./publish");
const WEB = { name: "Shop.Web", path: "/s/Web/Web.csproj", dir: "/s/Web", folders: [] };

describe("publish (BLD-6)", () => {
  it("offers the profiles, a folder and a container image", async () => {
    pick = "FolderProfile";
    await publishProject(WEB);
    expect(choices).toEqual(["FolderProfile (bin/publish)", "Folder…", "Container Image…"]);
    expect(targets.at(-1)).toEqual({ kind: "profile", name: "FolderProfile", configuration: "Release" });
    expect(runs.at(-1)).toMatchObject({ kind: "shell", name: "Publish: Shop.Web (FolderProfile)", cwd: "/s/Web" });
  });

  it("publishes a container image under the name given", async () => {
    pick = "Container";
    answer = " shop-web ";
    await publishProject(WEB);
    expect(targets.at(-1)).toEqual({ kind: "container", configuration: "Release", repository: "shop-web", tag: null });
  });

  it("does nothing when a step is cancelled", async () => {
    const before = runs.length;
    pick = "Folder…";
    answer = null;
    await publishProject(WEB);
    expect(runs.length).toBe(before);
  });

  it("names images after the project", () => {
    expect(repositoryName("Shop.Web")).toBe("shop-web");
    expect(repositoryName("My Api_2")).toBe("my-api_2");
  });
});

describe("run in container (PLT-5)", () => {
  it("runs the publish-then-run line in the Run window", async () => {
    answer = "shop-web";
    await runInContainer(WEB);
    expect(runs.at(-1)).toMatchObject({
      kind: "shell",
      name: "Container: Shop.Web",
      command: "dotnet publish x && docker run --rm -i shop-web:dev",
      args: [],
    });
  });
});
