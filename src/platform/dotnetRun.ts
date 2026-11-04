/**
 * `dotnet run` command lines, and `dotnet watch … run` ones (Hot Reload): what run
 * configurations and the debugger read from them.
 */

export interface DotnetRun {
  /** `dotnet watch --non-interactive run`: Hot Reload, restarting on edits it cannot apply. */
  watch: boolean;
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

/** Options of `dotnet watch` itself, before its `run`. */
const WATCH_OPTIONS = new Set(["--non-interactive", "--no-hot-reload", "-q", "--quiet", "-v", "--verbose"]);

/** `["run", "--project", "a.csproj", …]` → its parts; `null` when it is not `dotnet run` (or `dotnet watch run`). */
export function parseDotnetRun(command: string, args: readonly string[]): DotnetRun | null {
  const name = command
    .split(/[\\/]/)
    .pop()
    ?.replace(/\.exe$/i, "");
  if (name !== "dotnet") return null;
  let start = 1;
  const watch = args[0] === "watch";
  if (watch) {
    while (start < args.length && WATCH_OPTIONS.has(args[start]!)) start++;
    // `dotnet watch` runs the project when no command is given.
    if (args[start] === "run") start++;
  } else if (args[0] !== "run") return null;
  const out: DotnetRun = {
    watch,
    project: "",
    launchProfile: "",
    framework: "",
    configuration: "",
    programArgs: [],
    options: [],
  };
  for (let i = start; i < args.length; i++) {
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
    ...(run.watch ? ["watch", "--non-interactive", "run"] : ["run"]),
    ...(run.project ? ["--project", run.project] : []),
    ...(run.launchProfile ? ["--launch-profile", run.launchProfile] : []),
    ...(run.framework ? ["--framework", run.framework] : []),
    ...(run.configuration ? ["--configuration", run.configuration] : []),
    ...(run.options ?? []),
    ...(run.programArgs?.length ? ["--", ...run.programArgs] : []),
  ];
}
