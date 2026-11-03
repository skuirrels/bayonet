/** `dotnet run` command lines: what run configurations and the debugger read from them. */

export interface DotnetRun {
  /** `--project` value as written; empty = the project in the working directory. */
  project: string;
  launchProfile: string;
  /** `-f` / `--framework`: the target framework of a multi-targeting project. */
  framework: string;
  /** `-c` / `--configuration`; empty = Debug. */
  configuration: string;
  /** Arguments after `--`, for the program. */
  programArgs: string[];
  /** Other `dotnet run` options, kept as written. */
  options: string[];
}

const VALUED: Record<string, keyof Pick<DotnetRun, "project" | "launchProfile" | "framework" | "configuration">> = {
  "--project": "project",
  "-p": "project",
  "--launch-profile": "launchProfile",
  "-lp": "launchProfile",
  "--framework": "framework",
  "-f": "framework",
  "--configuration": "configuration",
  "-c": "configuration",
};

/** `["run", "--project", "a.csproj", …]` → its parts; `null` when it is not `dotnet run`. */
export function parseDotnetRun(command: string, args: readonly string[]): DotnetRun | null {
  const name = command
    .split(/[\\/]/)
    .pop()
    ?.replace(/\.exe$/i, "");
  if (name !== "dotnet" || args[0] !== "run") return null;
  const out: DotnetRun = {
    project: "",
    launchProfile: "",
    framework: "",
    configuration: "",
    programArgs: [],
    options: [],
  };
  for (let i = 1; i < args.length; i++) {
    const a = args[i]!;
    if (a === "--") {
      out.programArgs = args.slice(i + 1);
      break;
    }
    const eq = a.indexOf("=");
    const flag = eq > 0 && a.startsWith("-") ? a.slice(0, eq) : a;
    const key = VALUED[flag];
    if (key) {
      out[key] = eq > 0 && flag !== a ? a.slice(eq + 1) : (args[++i] ?? "");
    } else {
      out.options.push(a);
    }
  }
  return out;
}

/** The command line of `run` (inverse of {@link parseDotnetRun}). */
export function dotnetRunArgs(run: Partial<DotnetRun>): string[] {
  return [
    "run",
    ...(run.project ? ["--project", run.project] : []),
    ...(run.launchProfile ? ["--launch-profile", run.launchProfile] : []),
    ...(run.framework ? ["--framework", run.framework] : []),
    ...(run.configuration ? ["--configuration", run.configuration] : []),
    ...(run.options ?? []),
    ...(run.programArgs?.length ? ["--", ...run.programArgs] : []),
  ];
}
