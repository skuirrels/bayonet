import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AttachableProcess, LaunchConfig } from "@/platform/ipc";

const backend = vi.hoisted(() => ({
  hosts: {} as Record<string, unknown[]>,
  prepared: [] as string[],
  started: [] as unknown[],
}));

vi.mock("@/platform/ipc", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/platform/ipc")>();
  const ok = (data: unknown) => Promise.resolve({ status: "ok" as const, data });
  return {
    ...real,
    commands: {
      ...real.commands,
      debugAttachableProcesses: () => ok(backend.hosts[""] ?? []),
      debugSshProcesses: (ssh: string) => ok(backend.hosts[ssh] ?? []),
      debugPrepareSsh: (ssh: string) => {
        backend.prepared.push(ssh);
        return ok(
          JSON.stringify({ sshConfiguration: ssh, debuggerPath: "/home/me/.openidea/netcoredbg/linux-x64/netcoredbg" }),
        );
      },
    },
  };
});

vi.mock("../state", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../state")>()),
  startSession: (launch: LaunchConfig) => {
    backend.started.push(launch);
    return Promise.resolve();
  },
}));

import { useProject } from "@/platform/project";
import { provideNotificationService } from "@/workbench/contributions";
import { DebuggerOverlays } from "../ui/Overlays";
import { useOverlays } from "../ui/popoverStore";

const proc = (pid: number, name: string, ssh: string | null): AttachableProcess => ({
  pid,
  name,
  commandLine: `dotnet /srv/${name}.dll`,
  kind: "dotnet",
  adapter: "netcoredbg",
  host: null,
  port: null,
  container: null,
  ssh,
});

const notify = vi.fn();

beforeEach(() => {
  backend.hosts = { "": [proc(40, "Local.Api", null)], ssh1: [proc(4021, "Shop.Api", "ssh1")], ssh2: [] };
  backend.prepared.length = 0;
  backend.started.length = 0;
  notify.mockClear();
  provideNotificationService({ notify });
  useProject.setState({
    project: { root: "/proj" } as never,
    settings: {
      ssh: {
        configs: [
          { id: "ssh1", name: "Staging", host: "box", user: "me", auth: "openSsh" },
          { id: "ssh2", name: "Empty", host: "idle", user: "me", auth: "openSsh" },
        ],
      },
    } as never,
  });
});

afterEach(() => {
  act(() => useOverlays.setState({ attach: null }));
  useProject.setState({ project: null, settings: null });
});

async function chooseTarget(name: RegExp) {
  fireEvent.click(screen.getByRole("button", { name: "Where the process runs" }));
  fireEvent.click(await screen.findByRole("option", { name }));
}

describe("Attach to Process on an SSH host", () => {
  it("lists the chosen host's .NET processes and attaches through the debugger there", async () => {
    render(<DebuggerOverlays />);
    act(() =>
      useOverlays.setState({
        attach: { processes: backend.hosts[""] as AttachableProcess[], error: null, target: "" },
      }),
    );
    expect(await screen.findByText("Local.Api")).toBeInTheDocument();

    await chooseTarget(/Staging/);
    expect(await screen.findByText("Shop.Api")).toBeInTheDocument();
    expect(screen.getByText("Staging, pid 4021")).toBeInTheDocument();
    expect(screen.queryByText("Local.Api")).not.toBeInTheDocument();
    expect(useOverlays.getState().attach?.target).toBe("ssh1");

    fireEvent.keyDown(screen.getByRole("textbox", { name: "Filter processes" }), { key: "Enter" });
    await waitFor(() => expect(backend.started).toHaveLength(1));
    expect(backend.prepared).toEqual(["ssh1"]);
    expect(useOverlays.getState().attach).toBeNull();
    const launch = backend.started[0] as LaunchConfig;
    expect(launch).toMatchObject({
      name: "Shop.Api (Staging)",
      adapter: "netcoredbg",
      request: "attach",
      cwd: "/proj",
    });
    expect(JSON.parse(launch.extraJson!)).toMatchObject({
      processId: 4021,
      pipeTransport: { sshConfiguration: "ssh1" },
    });
    expect(notify).toHaveBeenCalledWith(expect.objectContaining({ title: "Preparing the debugger on Staging…" }));
  });

  it("says so when the host runs no .NET process, and returns to this machine", async () => {
    render(<DebuggerOverlays />);
    act(() => useOverlays.setState({ attach: { processes: [], error: null, target: "" } }));
    await chooseTarget(/Empty/);
    expect(await screen.findByText("No .NET processes are running on this host.")).toBeInTheDocument();

    await chooseTarget(/Local machine/);
    expect(await screen.findByText("Local.Api")).toBeInTheDocument();
    expect(useOverlays.getState().attach?.target).toBe("");
  });
});
