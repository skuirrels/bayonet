/**
 * .NET debugging on a Linux machine over SSH (DBG-8): the hosts Attach to Process offers,
 * a host's processes and debugger with the connection remedies (trust the host key, ask
 * for the password), and the `.NET Remote (SSH)` run configuration that repeats an attach.
 * netcoredbg runs on the host, on the SSH configuration's connection.
 */
import { tx } from "@/platform/i18n";
import { call, commands, IpcError, type AttachableProcess, type LaunchConfig, type RunConfig } from "@/platform/ipc";
import { sshLabel, type SshConfigValid } from "@/platform/remoteSettings";
import { workflowSettings } from "@/platform/workflowSettings";
import type { RunConfigTypeContribution } from "@/workbench/contributions";

export const DOTNET_SSH = "dotnet-remote-ssh";

/** Attach to Process targets: this machine, an SSH configuration's id, or the way to edit them. */
export const LOCAL_TARGET = "";
export const CONFIGURE_SSH = "<configure>";

type Configs = readonly SshConfigValid[];
const sshConfigs = (): Configs => workflowSettings().ssh.configs;

const displayName = (c: SshConfigValid) => c.name.trim() || sshLabel(c) || c.id;

/** Display name of the host of SSH configuration `id` (the id when the configuration is gone). */
export function sshHostName(id: string, configs: Configs = sshConfigs()): string {
  const c = configs.find((x) => x.id === id);
  return c ? displayName(c) : id;
}

export interface AttachTarget {
  value: string;
  label: string;
  detail?: string;
  icon?: string;
}

/** Where Attach to Process can look: this machine, then every SSH configuration with a host. */
export function attachTargets(configs: Configs = sshConfigs()): AttachTarget[] {
  const hosts = configs
    .filter((c) => c.host.trim())
    .map((c) => {
      const label = sshLabel(c);
      return { value: c.id, label: displayName(c), detail: c.name.trim() && label !== c.name ? label : undefined };
    });
  return [
    { value: LOCAL_TARGET, label: "Local machine" },
    ...hosts,
    { value: CONFIGURE_SSH, label: "Edit SSH Configurations…", icon: "settings" },
  ];
}

// ---------------------------------------------------------------------------
// Connecting

export type SshRemedy = "trustHost" | "secret" | null;

/** What a failed SSH connection needs from the user (`auth`: how the configuration authenticates). */
export function sshRemedy(message: string, auth: string): SshRemedy {
  if (/^Unknown SSH host key/.test(message)) return "trustHost";
  if (/^SSH (password|key passphrase) required|^cannot decrypt SSH key/.test(message)) return "secret";
  // With the OpenSSH configuration and agent there is nothing to type unless a key is encrypted.
  if (/^SSH authentication failed/.test(message)) return auth === "openSsh" ? null : "secret";
  return null;
}

export interface SshRemedyDeps {
  confirmTrust(message: string): Promise<boolean>;
  askSecret(message: string, save: boolean): Promise<{ secret: string; save: boolean } | null>;
  trustHost(ssh: string): Promise<unknown>;
  setSecret(ssh: string, secret: string, save: boolean): Promise<unknown>;
  configs(): Configs;
}

const defaultDeps: SshRemedyDeps = {
  confirmTrust: async (message) => (await import("./ui/SshPrompts")).confirmHostKey(message),
  askSecret: async (message, save) => (await import("./ui/SshPrompts")).askSshSecret(message, save),
  trustHost: (ssh) => call(commands.sshTrustHost(ssh)),
  setSecret: (ssh, secret, save) => commands.sshSetSecret(ssh, secret, save),
  configs: sshConfigs,
};

/**
 * Runs work on the host of SSH configuration `ssh`; when the connection needs its host key
 * trusted or a password, asks and retries. Other errors, and declined remedies, rethrow.
 */
export async function withSshRemedies<T>(
  ssh: string,
  run: () => Promise<T>,
  deps: SshRemedyDeps = defaultDeps,
): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await run();
    } catch (e) {
      if (attempt >= 3 || !(e instanceof IpcError)) throw e;
      const config = deps.configs().find((c) => c.id === ssh);
      const remedy = sshRemedy(e.message, config?.auth ?? "openSsh");
      if (remedy === "trustHost") {
        if (!(await deps.confirmTrust(e.message))) throw e;
        await deps.trustHost(ssh);
      } else if (remedy === "secret") {
        const answer = await deps.askSecret(e.message, config?.saveSecret ?? false);
        if (!answer) throw e;
        await deps.setSecret(ssh, answer.secret, answer.save);
      } else throw e;
    }
  }
}

/** The .NET processes running on the host of SSH configuration `ssh`. */
export function sshProcesses(ssh: string, deps?: SshRemedyDeps): Promise<AttachableProcess[]> {
  return withSshRemedies(ssh, () => call(commands.debugSshProcesses(ssh)), deps);
}

/** Installs netcoredbg on the host when it has none; the `pipeTransport` of an attach request. */
export async function prepareSshDebugger(ssh: string, deps?: SshRemedyDeps): Promise<Record<string, unknown>> {
  return JSON.parse(await withSshRemedies(ssh, () => call(commands.debugPrepareSsh(ssh)), deps));
}

// ---------------------------------------------------------------------------
// .NET Remote (SSH) run configuration

export const dotnetSshType: RunConfigTypeContribution = {
  id: DOTNET_SSH,
  label: ".NET Remote (SSH)",
  icon: "file-csharp",
  description: "Attach to a .NET process on a Linux machine over SSH",
  debugOnly: true,
  debugAdapter: "netcoredbg",
  fields: [
    {
      key: "ssh",
      label: "SSH configuration",
      control: "select",
      required: true,
      options: [{ value: "", label: "Choose an SSH configuration" }],
      loadOptions: async () => attachTargets().filter((t) => t.value !== LOCAL_TARGET && t.value !== CONFIGURE_SSH),
      hint: "SSH configurations are edited in Settings › Tools › SSH Configurations. The first attach installs netcoredbg on the host.",
    },
    {
      key: "process",
      label: "Process",
      mono: true,
      required: true,
      placeholder: "Process id or name",
      hint: "A process id, or the name of the .NET app (as Attach to Process lists it) when exactly one such process runs on the host",
    },
  ],
  commandLine: () => null,
};

/** What a `.NET Remote (SSH)` configuration names: a process id, else an app name. */
export function processQuery(value: string): { pid: number } | { name: string } {
  const v = value.trim();
  return /^\d+$/.test(v) ? { pid: Number(v) } : { name: v.replace(/\.(dll|exe)$/i, "") };
}

/** The one process named `name` (ignoring case) on `host`; throws when none or several are. */
export function processNamed(list: readonly AttachableProcess[], name: string, host: string): AttachableProcess {
  const found = list.filter((p) => p.name.toLowerCase() === name.toLowerCase());
  if (found.length === 1) return found[0]!;
  if (!found.length) throw new Error(tx`No .NET process named ${name} is running on ${host}`);
  const pids = found.map((p) => p.pid).join(", ");
  throw new Error(
    tx`${found.length} .NET processes named ${name} are running on ${host} (process ids ${pids}). Enter a process id in the configuration.`,
  );
}

/**
 * The attach request of a `.NET Remote (SSH)` configuration: finds the process (by name
 * when no id is given), makes sure the host has netcoredbg, and attaches through it.
 */
export async function dotnetSshAttachConfig(rc: RunConfig, root: string, deps?: SshRemedyDeps): Promise<LaunchConfig> {
  const params = rc.params ?? {};
  const ssh = params.ssh?.trim() ?? "";
  const configs = deps?.configs() ?? sshConfigs();
  if (!configs.some((c) => c.id === ssh))
    throw new Error(tx`'${rc.name}' has no SSH configuration to connect with. Choose one in the configuration.`);
  const query = processQuery(params.process ?? "");
  const host = sshHostName(ssh, configs);
  const pid = "pid" in query ? query.pid : processNamed(await sshProcesses(ssh, deps), query.name, host).pid;
  const pipeTransport = await prepareSshDebugger(ssh, deps);
  return {
    name: rc.name,
    adapter: "netcoredbg",
    request: "attach",
    program: null,
    args: [],
    cwd: root,
    env: {},
    extraJson: JSON.stringify({
      processId: pid,
      justMyCode: workflowSettings().debugger.skipLibraryCode,
      pipeTransport,
    }),
  };
}
