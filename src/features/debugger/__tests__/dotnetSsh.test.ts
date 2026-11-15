import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AttachableProcess, RunConfig } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { registerRunConfigType } from "@/workbench/contributions";

/** What the fake backend answers `debugSshProcesses` / `debugPrepareSsh` with, in order. */
const answers = vi.hoisted(() => ({
  processes: [] as Array<unknown>,
  prepare: [] as Array<unknown>,
  calls: [] as string[],
}));

vi.mock("@/platform/ipc", async (orig) => {
  const real = await orig<typeof import("@/platform/ipc")>();
  const reply = (queue: unknown[], fallback: unknown) => {
    const next = queue.length ? queue.shift() : fallback;
    return Promise.resolve(
      next && typeof next === "object" && "error" in next
        ? { status: "error" as const, error: (next as { error: unknown }).error }
        : { status: "ok" as const, data: next },
    );
  };
  return {
    ...real,
    commands: {
      ...real.commands,
      debugSshProcesses: (ssh: string) => {
        answers.calls.push(`processes:${ssh}`);
        return reply(answers.processes, []);
      },
      debugPrepareSsh: (ssh: string) => {
        answers.calls.push(`prepare:${ssh}`);
        return reply(
          answers.prepare,
          JSON.stringify({ sshConfiguration: ssh, debuggerPath: "/home/me/.openidea/netcoredbg/linux-x64/netcoredbg" }),
        );
      },
    },
  };
});

const {
  attachTargets,
  CONFIGURE_SSH,
  DOTNET_SSH,
  dotnetSshAttachConfig,
  dotnetSshType,
  LOCAL_TARGET,
  processNamed,
  processQuery,
  sshHostName,
  sshProcesses,
  sshRemedy,
  withSshRemedies,
} = await import("../dotnetSsh");
const { attachConfig, attachDetail, attachRows } = await import("../attach");
const { loadAttachProcesses } = await import("../attachPopup");
const { useOverlays } = await import("../ui/popoverStore");
const { missingParams, newConfigOfType, isDebugOnly, withParams } = await import("@/features/run/types");

const box = { id: "ssh1", name: "Staging", host: "box", port: 2200, user: "me", auth: "password", saveSecret: true };
const agent = { id: "ssh2", name: "", host: "build.example.com", port: null, user: "", auth: "openSsh" };
const draft = { id: "ssh3", name: "Not yet", host: "", port: null, user: "", auth: "openSsh" };

const proc = (pid: number, name: string, ssh = "ssh1"): AttachableProcess => ({
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

const error = (kind: string, message: string) => ({ error: { kind, message, data: null } });

function deps(over: Partial<Parameters<typeof withSshRemedies>[2]> = {}) {
  return {
    confirmTrust: vi.fn(async () => true),
    askSecret: vi.fn(async () => ({ secret: "hunter2", save: true }) as { secret: string; save: boolean } | null),
    trustHost: vi.fn(async () => undefined),
    setSecret: vi.fn(async () => undefined),
    configs: () => [box, agent, draft] as never,
    ...over,
  };
}

beforeEach(() => {
  answers.processes.length = 0;
  answers.prepare.length = 0;
  answers.calls.length = 0;
  useProject.setState({
    settings: { ssh: { configs: [box, agent, draft] }, debugger: { skipLibraryCode: true } } as never,
  });
});
afterEach(() => {
  useProject.setState({ settings: null });
  useOverlays.setState({ attach: null });
});

describe("Attach to Process targets", () => {
  it("offers this machine, every SSH configuration with a host, then the settings", () => {
    expect(attachTargets()).toEqual([
      { value: LOCAL_TARGET, label: "Local machine" },
      { value: "ssh1", label: "Staging", detail: "me@box:2200" },
      { value: "ssh2", label: "build.example.com", detail: undefined },
      { value: CONFIGURE_SSH, label: "Edit SSH Configurations…", icon: "settings" },
    ]);
    expect(attachTargets([]).map((t) => t.value)).toEqual([LOCAL_TARGET, CONFIGURE_SSH]);
  });

  it("names a host by its configuration, or by the id when that is gone", () => {
    expect(sshHostName("ssh1")).toBe("Staging");
    expect(sshHostName("ssh2")).toBe("build.example.com");
    expect(sshHostName("removed")).toBe("removed");
  });

  it("lists a host's processes in the popup and drops the answer of a target left meanwhile", async () => {
    useOverlays.setState({ attach: { processes: [], error: null, target: LOCAL_TARGET } });
    answers.processes.push([proc(4021, "Shop.Api")]);
    const loading = loadAttachProcesses("ssh1");
    expect(useOverlays.getState().attach).toEqual({ processes: null, error: null, target: "ssh1" });
    await loading;
    expect(useOverlays.getState().attach?.processes?.map((p) => p.pid)).toEqual([4021]);

    // An unreachable host shows why; nothing is listed.
    answers.processes.push(error("io", "cannot connect to SSH host build.example.com:22: refused"));
    await loadAttachProcesses("ssh2");
    expect(useOverlays.getState().attach).toEqual({
      processes: [],
      error: "cannot connect to SSH host build.example.com:22: refused",
      target: "ssh2",
    });

    // The first answer arrives after another target was chosen: it is not shown.
    answers.processes.push([proc(1, "Old")], [proc(2, "New", "ssh2")]);
    const first = loadAttachProcesses("ssh1");
    const second = loadAttachProcesses("ssh2");
    await Promise.all([first, second]);
    expect(useOverlays.getState().attach).toMatchObject({ target: "ssh2" });
    expect(useOverlays.getState().attach?.processes?.map((p) => p.name)).toEqual(["New"]);

    // Closed while loading: stays closed.
    const late = loadAttachProcesses("ssh1");
    useOverlays.setState({ attach: null });
    await late;
    expect(useOverlays.getState().attach).toBeNull();
  });

  it("attaches to a process on an SSH host through the debugger installed there", async () => {
    const p = proc(4021, "Shop.Api");
    expect(attachDetail(p)).toBe("Staging, pid 4021");
    expect(attachRows([p], "").map((r) => r.key)).toEqual(["group:dotnet", "dotnet:Staging:4021"]);
    const config = await attachConfig(p, "/proj");
    expect(answers.calls).toEqual(["prepare:ssh1"]);
    expect(config).toMatchObject({
      name: "Shop.Api (Staging)",
      adapter: "netcoredbg",
      request: "attach",
      cwd: "/proj",
    });
    expect(JSON.parse(config.extraJson!)).toEqual({
      processId: 4021,
      justMyCode: true,
      pipeTransport: {
        sshConfiguration: "ssh1",
        debuggerPath: "/home/me/.openidea/netcoredbg/linux-x64/netcoredbg",
      },
    });
  });
});

describe("SSH connection remedies", () => {
  it("reads what a failed connection needs", () => {
    expect(sshRemedy("Unknown SSH host key for box:22 (ED25519 SHA256:abc)", "password")).toBe("trustHost");
    expect(sshRemedy("SSH password required for me@box:22", "password")).toBe("secret");
    expect(sshRemedy("SSH key passphrase required for /keys/id", "keyPair")).toBe("secret");
    expect(sshRemedy("cannot decrypt SSH key /keys/id (wrong passphrase?): bad", "keyPair")).toBe("secret");
    // A wrong saved password can be retyped; with the agent there is nothing to type.
    expect(sshRemedy("SSH authentication failed for me@box:22", "password")).toBe("secret");
    expect(sshRemedy("SSH authentication failed for me@box:22", "openSsh")).toBeNull();
    expect(sshRemedy("SSH host key of box:22 has changed (known_hosts line 3)", "password")).toBeNull();
    expect(
      sshRemedy("Remote .NET debugging needs a Linux host (x64 or arm64); me@box:22 runs macOS", "password"),
    ).toBeNull();
  });

  it("trusts the host key, asks for the password, then lists the processes", async () => {
    answers.processes.push(
      error("protocol", "Unknown SSH host key for box:2200 (ED25519 SHA256:abc)"),
      error("permissionDenied", "SSH password required for me@box:2200"),
      [proc(4021, "Shop.Api")],
    );
    const d = deps();
    expect((await sshProcesses("ssh1", d)).map((p) => p.pid)).toEqual([4021]);
    expect(d.confirmTrust).toHaveBeenCalledWith("Unknown SSH host key for box:2200 (ED25519 SHA256:abc)");
    expect(d.trustHost).toHaveBeenCalledWith("ssh1");
    // The dialog starts from the configuration's own "save" choice.
    expect(d.askSecret).toHaveBeenCalledWith("SSH password required for me@box:2200", true);
    expect(d.setSecret).toHaveBeenCalledWith("ssh1", "hunter2", true);
    expect(answers.calls).toEqual(["processes:ssh1", "processes:ssh1", "processes:ssh1"]);
  });

  it("gives up when the user declines, on other errors, and after repeated failures", async () => {
    answers.processes.push(error("protocol", "Unknown SSH host key for box:2200 (x)"));
    const declined = deps({ confirmTrust: vi.fn(async () => false) });
    await expect(sshProcesses("ssh1", declined)).rejects.toThrow("Unknown SSH host key");
    expect(declined.trustHost).not.toHaveBeenCalled();

    answers.processes.push(error("permissionDenied", "SSH password required for me@box:2200"));
    const cancelled = deps({ askSecret: vi.fn(async () => null) });
    await expect(sshProcesses("ssh1", cancelled)).rejects.toThrow("SSH password required");
    expect(cancelled.setSecret).not.toHaveBeenCalled();

    answers.processes.push(
      error("unavailable", "Remote .NET debugging needs a Linux host (x64 or arm64); box runs macOS"),
    );
    const other = deps();
    await expect(sshProcesses("ssh1", other)).rejects.toThrow("needs a Linux host");
    expect(other.askSecret).not.toHaveBeenCalled();

    // A password the host keeps refusing is not asked for forever.
    for (let i = 0; i < 6; i++)
      answers.processes.push(error("permissionDenied", "SSH authentication failed for me@box:2200"));
    const wrong = deps();
    await expect(sshProcesses("ssh1", wrong)).rejects.toThrow("SSH authentication failed");
    expect(wrong.askSecret).toHaveBeenCalledTimes(3);
  });
});

describe(".NET Remote (SSH) configuration", () => {
  let dispose: () => void;
  beforeEach(() => {
    dispose = registerRunConfigType(dotnetSshType);
  });
  afterEach(() => dispose());

  const configOf = (params: Record<string, string>): RunConfig => {
    const c = newConfigOfType(dotnetSshType, []);
    return withParams({ ...c, name: "Shop on staging" }, { ...c.params, ...params }, dotnetSshType);
  };

  it("is debug-only, runs netcoredbg and needs a host and a process", async () => {
    const c = configOf({});
    expect(c.kind).toBe(DOTNET_SSH);
    expect(isDebugOnly(c)).toBe(true);
    expect(c.debugAdapter).toBe("netcoredbg");
    expect(c.command).toBe("");
    expect(dotnetSshType.commandLine({})).toBeNull();
    expect(missingParams(c, dotnetSshType)).toEqual({
      ssh: "SSH configuration is required",
      process: "Process is required",
    });
    expect(missingParams(configOf({ ssh: "ssh1", process: "Shop.Api" }), dotnetSshType)).toEqual({});
    // The host list: the saved configurations with a host.
    const ssh = dotnetSshType.fields.find((f) => f.key === "ssh")!;
    expect((await ssh.loadOptions!()).map((o) => o.value)).toEqual(["ssh1", "ssh2"]);
  });

  it("reads a process id or an app name", () => {
    expect(processQuery(" 4021 ")).toEqual({ pid: 4021 });
    expect(processQuery("Shop.Api")).toEqual({ name: "Shop.Api" });
    expect(processQuery("Shop.Api.dll")).toEqual({ name: "Shop.Api" });
    const list = [proc(1, "Shop.Api"), proc(2, "Worker"), proc(3, "Worker")];
    expect(processNamed(list, "shop.api", "Staging").pid).toBe(1);
    expect(() => processNamed(list, "Billing", "Staging")).toThrow(
      "No .NET process named Billing is running on Staging",
    );
    expect(() => processNamed(list, "Worker", "Staging")).toThrow(
      "2 .NET processes named Worker are running on Staging (process ids 2, 3). Enter a process id in the configuration.",
    );
  });

  it("attaches by process id without listing the host's processes", async () => {
    const launch = await dotnetSshAttachConfig(configOf({ ssh: "ssh1", process: "4021" }), "/proj", deps());
    expect(answers.calls).toEqual(["prepare:ssh1"]);
    expect(launch).toMatchObject({ name: "Shop on staging", adapter: "netcoredbg", request: "attach", cwd: "/proj" });
    expect(JSON.parse(launch.extraJson!)).toEqual({
      processId: 4021,
      justMyCode: true,
      pipeTransport: {
        sshConfiguration: "ssh1",
        debuggerPath: "/home/me/.openidea/netcoredbg/linux-x64/netcoredbg",
      },
    });
  });

  it("finds the process by name, asking for the password first when the host needs it", async () => {
    answers.processes.push(error("permissionDenied", "SSH password required for me@box:2200"), [
      proc(7, "Worker"),
      proc(4021, "Shop.Api"),
    ]);
    const d = deps();
    const launch = await dotnetSshAttachConfig(configOf({ ssh: "ssh1", process: "Shop.Api" }), "/proj", d);
    expect(d.setSecret).toHaveBeenCalledWith("ssh1", "hunter2", true);
    expect(answers.calls).toEqual(["processes:ssh1", "processes:ssh1", "prepare:ssh1"]);
    expect(JSON.parse(launch.extraJson!).processId).toBe(4021);
  });

  it("fails clearly when the process, the configuration or a Linux host is missing", async () => {
    answers.processes.push([proc(7, "Worker")]);
    await expect(dotnetSshAttachConfig(configOf({ ssh: "ssh1", process: "Shop.Api" }), "/p", deps())).rejects.toThrow(
      "No .NET process named Shop.Api is running on Staging",
    );
    await expect(dotnetSshAttachConfig(configOf({ ssh: "removed", process: "1" }), "/p", deps())).rejects.toThrow(
      "'Shop on staging' has no SSH configuration to connect with",
    );
    answers.prepare.push(
      error("unavailable", "Remote .NET debugging needs a Linux host (x64 or arm64); me@box:2200 runs macOS"),
    );
    await expect(dotnetSshAttachConfig(configOf({ ssh: "ssh1", process: "1" }), "/p", deps())).rejects.toThrow(
      "runs macOS",
    );
    // Nothing was prepared for the first two.
    expect(answers.calls).toEqual(["processes:ssh1", "prepare:ssh1"]);
  });
});
