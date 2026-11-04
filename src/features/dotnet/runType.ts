/** The ".NET Project" run configuration type: `dotnet run` with a launch profile. */
import { formatArgs, parseArgs } from "@/platform/commandLine";
import { dotnetRunArgs, parseDotnetRun } from "@/platform/dotnetRun";
import type { RunConfig } from "@/platform/ipc";
import type { RunConfigTypeContribution } from "@/workbench/contributions";

export const DOTNET_RUN_KIND = "dotnet";

/** Values of `key` in the detected `dotnet run` configurations. */
function detected(configs: readonly RunConfig[], key: "project" | "launchProfile" | "framework"): string[] {
  const values = configs.map((c) => parseDotnetRun(c.command, c.args)?.[key]).filter((v): v is string => !!v);
  return [...new Set(values)];
}

export const dotnetRunType: RunConfigTypeContribution = {
  id: DOTNET_RUN_KIND,
  label: ".NET Project",
  icon: "project-csharp",
  description: "Run a .NET project with dotnet run and a launch profile",
  debugAdapter: "netcoredbg",
  fields: [
    {
      key: "project",
      label: "Project",
      mono: true,
      required: true,
      placeholder: "Path to the .csproj, .fsproj or .vbproj",
      suggestions: (configs) => detected(configs, "project"),
    },
    {
      key: "launchProfile",
      label: "Launch profile",
      mono: true,
      placeholder: "First profile in Properties/launchSettings.json",
      suggestions: (configs) => detected(configs, "launchProfile"),
    },
    {
      key: "framework",
      label: "Target framework",
      mono: true,
      placeholder: "The project's first target framework",
      hint: "e.g. net10.0; only needed for a project with several target frameworks",
      suggestions: (configs) => detected(configs, "framework"),
    },
    {
      key: "configuration",
      label: "Configuration",
      control: "select",
      options: [
        { value: "", label: "Debug" },
        { value: "Release", label: "Release" },
      ],
    },
    { key: "arguments", label: "Program arguments", mono: true },
    {
      key: "hotReload",
      label: "Hot Reload (dotnet watch)",
      control: "checkbox",
      hint: "Applies code edits to the running app; edits it cannot apply restart it. Debugging runs without it.",
    },
  ],
  commandLine: (p) => ({
    command: p.executable || "dotnet",
    args: dotnetRunArgs({
      watch: p.hotReload === "true",
      project: p.project?.trim(),
      launchProfile: p.launchProfile?.trim(),
      framework: p.framework?.trim(),
      configuration: p.configuration?.trim(),
      programArgs: parseArgs(p.arguments ?? ""),
    }),
  }),
  paramsFrom(c) {
    const run = parseDotnetRun(c.command, c.args);
    if (!run || run.options.length) return null;
    return {
      project: run.project,
      launchProfile: run.launchProfile,
      framework: run.framework,
      configuration: run.configuration,
      arguments: formatArgs(run.programArgs),
      hotReload: run.watch ? "true" : "false",
      ...(c.command !== "dotnet" ? { executable: c.command } : {}),
    };
  },
};
