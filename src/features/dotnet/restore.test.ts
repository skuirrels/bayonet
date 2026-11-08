import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RestoreResult } from "@/bindings";
import { IpcError } from "@/platform/ipc";
import { setContext } from "@/platform/context";
import { useProject } from "@/platform/project";
import { provideNotificationService, type NotificationService } from "@/workbench/contributions";
import { useTasks } from "@/workbench/tasks";
import {
  RESTORE_TASK_ID,
  resetRestore,
  restore,
  restoreAgain,
  restoreIfNeeded,
  setRestoreBackend,
  touchesRestoreInputs,
  watchBuilds,
  type RestoreBackend,
} from "./restore";

type Note = Parameters<NotificationService["notify"]>[0];
let notes: Note[];

const OK: RestoreResult = { success: true, messages: [] };

/** A backend whose restores finish when the test resolves them, in order. */
function fakeBackend(stale: string[] = []) {
  const runs: { targets: string[]; clear: string[]; finish: (r: RestoreResult) => void; fail: (e: unknown) => void }[] =
    [];
  const backend: RestoreBackend = {
    needed: vi.fn(async (projects: string[]) => projects.filter((p) => stale.includes(p))),
    restore: vi.fn(
      (targets: string[], clear: string[]) =>
        new Promise<RestoreResult>((finish, fail) => runs.push({ targets, clear, finish, fail })),
    ),
    cancel: vi.fn(),
  };
  setRestoreBackend(backend);
  return { backend, runs };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const task = () => useTasks.getState().tasks.find((t) => t.id === RESTORE_TASK_ID);

beforeEach(() => {
  notes = [];
  provideNotificationService({ notify: (n) => void notes.push(n) });
  useProject.setState({ project: { root: "/w", trusted: true } as never, settings: null });
});

afterEach(() => {
  setContext("build.running", false);
  resetRestore();
  setRestoreBackend(null);
  useTasks.setState({ tasks: [] });
});

describe("restore queue", () => {
  it("runs one restore at a time and gathers what is asked for meanwhile", async () => {
    const { backend, runs } = fakeBackend();
    const first = restore(["/w/App/App.csproj"]);
    await flush();
    expect(task()).toMatchObject({ title: "Restoring NuGet packages", message: "App.csproj", done: false });

    void restore(["/w/Lib/Lib.csproj"]);
    void restore(["/w/Tests/Tests.csproj", "/w/Lib/Lib.csproj"]);
    expect(runs).toHaveLength(1);

    runs[0]!.finish(OK);
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App/App.csproj"], ["/w/Lib/Lib.csproj", "/w/Tests/Tests.csproj"]]);
    runs[1]!.finish(OK);
    await first;
    expect(backend.restore).toHaveBeenCalledTimes(2);
    expect(task()).toMatchObject({ done: true });
    expect(notes).toEqual([]);
  });

  it("notifies a failed restore and clears its diagnostics on the next one", async () => {
    const { runs } = fakeBackend();
    const done = restore(["/w/App.slnx"]);
    await flush();
    runs[0]!.finish({
      success: false,
      messages: [
        {
          severity: "error",
          code: "NU1301",
          text: "Unable to load the service index for source https://feed.example/v3/index.json.",
          project: "/w/App/App.csproj",
        },
        { severity: "warning", code: "NU1603", text: "Serilog 4.0.0 was not found.", project: "/w/Lib/Lib.csproj" },
      ],
    });
    await done;
    expect(notes).toHaveLength(1);
    expect(notes[0]).toMatchObject({
      level: "error",
      title: "NuGet restore failed: 1 error",
      body: "NU1301: Unable to load the service index for source https://feed.example/v3/index.json.",
    });
    expect(notes[0]!.actions!.map((a) => a.label)).toEqual(["Show Problems", "Open App.csproj"]);

    const again = restore(["/w/App.slnx"]);
    await flush();
    expect(runs[1]!.clear).toEqual(["/w/App/App.csproj", "/w/Lib/Lib.csproj"]);
    runs[1]!.finish(OK);
    await again;
    expect(notes).toHaveLength(1);
  });

  it("does not notify warnings or a cancelled restore", async () => {
    const { runs } = fakeBackend();
    const warned = restore(["/w/App/App.csproj"]);
    await flush();
    runs[0]!.finish({
      success: true,
      messages: [{ severity: "warning", code: "NU1603", text: "odd", project: "/w/App/App.csproj" }],
    });
    await warned;
    const cancelled = restore(["/w/App/App.csproj"]);
    await flush();
    runs[1]!.fail(new IpcError({ kind: "cancelled", message: "cancelled", data: null }));
    await cancelled;
    expect(notes).toEqual([]);
  });

  it("cancels through the background task", async () => {
    const { backend } = fakeBackend();
    void restore(["/w/App/App.csproj"]);
    await flush();
    task()!.cancel!();
    expect(backend.cancel).toHaveBeenCalledTimes(1);
  });
});

describe("restoreIfNeeded", () => {
  it("restores the solution when several projects are stale, else the one project", async () => {
    const { runs } = fakeBackend(["/w/App/App.csproj", "/w/Lib/Lib.csproj"]);
    void restoreIfNeeded(["/w/App/App.csproj", "/w/Lib/Lib.csproj", "/w/Ok/Ok.csproj"], "/w/App.slnx");
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App.slnx"]]);
    runs[0]!.finish(OK);
    await flush();

    void restoreIfNeeded(["/w/Lib/Lib.csproj", "/w/Ok/Ok.csproj"], "/w/App.slnx");
    await flush();
    expect(runs[1]!.targets).toEqual(["/w/Lib/Lib.csproj"]);
  });

  it("skips projects being restored and does nothing when none is stale", async () => {
    const { backend, runs } = fakeBackend(["/w/App/App.csproj"]);
    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    await flush();
    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    await flush();
    expect(runs).toHaveLength(1);

    await restoreIfNeeded(["/w/Ok/Ok.csproj"], "/w/App.slnx");
    expect(backend.restore).toHaveBeenCalledTimes(1);
  });

  it("is off when the setting is turned off and in Restricted Mode", async () => {
    const { backend } = fakeBackend(["/w/App/App.csproj"]);
    useProject.setState({ settings: { languages: { dotnet: { restoreOnOpen: false } } } as never });
    await restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    useProject.setState({ project: { root: "/w", trusted: false } as never, settings: null });
    await restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    expect(backend.needed).not.toHaveBeenCalled();
  });
});

describe("restore review cases", () => {
  const failed = (project: string): RestoreResult => ({
    success: false,
    messages: [{ severity: "error", code: "NU1101", text: "Unable to find package Nope.", project }],
  });

  it("clears only the diagnostics of what it restores", async () => {
    const { runs } = fakeBackend();
    void restore(["/w/App/App.csproj"]);
    await flush();
    runs[0]!.finish(failed("/w/App/App.csproj"));
    await flush();

    void restore(["/w/Lib/Lib.csproj"]);
    await flush();
    expect(runs[1]!.clear).toEqual([]);
    runs[1]!.finish(OK);
    await flush();

    void restore(["/w/App.slnx"]);
    await flush();
    expect(runs[2]!.clear).toEqual(["/w/App/App.csproj"]);
  });

  it("restores what Roslyn asks for once, though its restore looks current", async () => {
    const { runs } = fakeBackend();
    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    await flush();
    expect(runs).toHaveLength(0);

    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx", true);
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App/App.csproj"]]);
    runs[0]!.finish(OK);
    await flush();

    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx", true);
    await flush();
    expect(runs).toHaveLength(1);
  });

  it("restores again when the .NET environment changes, after a running build", async () => {
    const { backend, runs } = fakeBackend();
    const unwatch = watchBuilds();
    const projects = ["/w/App/App.csproj", "/w/Lib/Lib.csproj"];
    void restoreIfNeeded(projects, "/w/App.slnx", true);
    await flush();
    runs[0]!.finish(OK);
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App.slnx"]]);

    setContext("build.running", true);
    await restoreAgain(projects, "/w/App.slnx");
    expect(backend.needed).toHaveBeenCalledTimes(1);
    setContext("build.running", false);
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App.slnx"], ["/w/App.slnx"]]);
    unwatch();
  });

  it("does not queue a project again while its solution is queued", async () => {
    const { runs } = fakeBackend(["/w/App/App.csproj"]);
    void restore(["/w/Other/Other.csproj"]);
    void restore(["/w/App.slnx"]);
    await flush();
    void restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    await flush();
    runs[0]!.finish(OK);
    await flush();
    expect(runs[1]!.targets).toEqual(["/w/App.slnx"]);
  });

  it("waits for a running build to end", async () => {
    const { backend, runs } = fakeBackend(["/w/App/App.csproj"]);
    const unwatch = watchBuilds();
    setContext("build.running", true);
    await restoreIfNeeded(["/w/App/App.csproj"], "/w/App.slnx");
    expect(backend.needed).not.toHaveBeenCalled();
    setContext("build.running", false);
    await flush();
    expect(runs.map((r) => r.targets)).toEqual([["/w/App/App.csproj"]]);
    unwatch();
  });
});

describe("touchesRestoreInputs", () => {
  it("matches project files and the props above them, not restore output", () => {
    expect(touchesRestoreInputs(["/w/App/App.csproj"])).toBe(true);
    expect(touchesRestoreInputs(["/w/Directory.Packages.props"])).toBe(true);
    expect(touchesRestoreInputs(["/w/nuget.config"])).toBe(true);
    expect(touchesRestoreInputs(["/w/App/obj/App.csproj.nuget.g.props", "/w/App/obj/project.assets.json"])).toBe(false);
  });
});
