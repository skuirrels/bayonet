/**
 * Publish (BLD-6): `dotnet publish` of a project with one of its publish profiles
 * (`Properties/PublishProfiles/*.pubxml`), to a folder, or as a container image
 * (the SDK's `PublishContainer`), run in the Run tool window.
 */
import { commands, type PublishProfile, type PublishTarget } from "@/bindings";
import { executeAction } from "@/platform/actions";
import { tx } from "@/platform/i18n";
import { call, isTauri, type RunConfig } from "@/platform/ipc";
import { chooseItem, promptInput } from "@/workbench/inputDialogs";
import { notifications } from "@/workbench/contributions";
import type { SolutionProject } from "./projects/store";

export type PublishChoice =
  | { kind: "profile"; label: string; profile: PublishProfile }
  | { kind: "folder"; label: string }
  | { kind: "container"; label: string };

/** What the chooser lists: the profiles, then a folder and a container image. */
export function publishChoices(profiles: readonly PublishProfile[]): PublishChoice[] {
  const out: PublishChoice[] = profiles.map((profile) => {
    const where = [profile.method, profile.target].filter(Boolean).join(" → ");
    return { kind: "profile", label: where ? `${profile.name} (${where})` : profile.name, profile };
  });
  out.push({ kind: "folder", label: tx`Folder…` }, { kind: "container", label: tx`Container Image…` });
  return out;
}

/** A container repository name from a project name: lowercase, `.` and spaces as `-`. */
export function repositoryName(project: string): string {
  return project
    .toLowerCase()
    .replace(/[^a-z0-9._/-]+/g, "-")
    .replace(/\./g, "-")
    .replace(/^-+|-+$/g, "");
}

const REPOSITORY = /^[a-z0-9]+(?:[._-][a-z0-9]+)*(?:\/[a-z0-9]+(?:[._-][a-z0-9]+)*)*$/;

/** Asks for the image to build for `project`; `null` when cancelled. */
function askImageName(project: SolutionProject, title: string, okLabel: string): Promise<string | null> {
  return promptInput({
    title,
    label: tx`Image name:`,
    initial: repositoryName(project.name),
    okLabel,
    validate: (v) => (REPOSITORY.test(v.trim()) ? null : tx`Use lowercase letters, digits, '.', '_', '-' and '/'`),
    hint: tx`Builds the image with the SDK's container support and loads it into Docker or Podman.`,
  });
}

/** Asks where to publish `project`; `null` when cancelled. */
async function chooseTarget(project: SolutionProject): Promise<{ target: PublishTarget; label: string } | null> {
  const profiles = await call(commands.dotnetPublishProfiles(project.path)).catch(() => [] as PublishProfile[]);
  const choices = publishChoices(profiles);
  const name = project.name;
  const picked = await chooseItem({ title: tx`Publish ${name}`, items: choices.map((c) => c.label) });
  const choice = choices.find((c) => c.label === picked);
  if (!choice) return null;
  if (choice.kind === "profile")
    return {
      target: { kind: "profile", name: choice.profile.name, configuration: choice.profile.configuration },
      label: choice.profile.name,
    };
  if (choice.kind === "folder") {
    const output = await promptInput({
      title: tx`Publish to Folder`,
      label: tx`Folder:`,
      initial: `${project.dir}/bin/Release/publish`,
      okLabel: tx`Publish`,
    });
    if (!output) return null;
    return {
      target: { kind: "folder", output, configuration: "Release", runtime: null, selfContained: false },
      label: tx`Folder`,
    };
  }
  const repository = await askImageName(project, tx`Publish Container Image`, tx`Publish`);
  if (!repository) return null;
  return {
    target: { kind: "container", configuration: "Release", repository: repository.trim(), tag: null },
    label: tx`Container`,
  };
}

/** Publishes `project` where the user picks. */
export async function publishProject(project: SolutionProject): Promise<void> {
  if (!isTauri()) return;
  try {
    const chosen = await chooseTarget(project);
    if (!chosen) return;
    const cmd = await call(commands.dotnetPublishCommand(project.path, chosen.target));
    const config: RunConfig = {
      id: `publish:${project.path}`,
      name: `Publish: ${project.name} (${chosen.label})`,
      kind: "shell",
      command: cmd.command,
      args: cmd.args,
      cwd: cmd.cwd,
      env: { DOTNET_CLI_TELEMETRY_OPTOUT: "1", DOTNET_NOLOGO: "1" },
      detected: true,
      isTest: false,
      debugAdapter: null,
    };
    await executeAction("run.runConfig", config);
  } catch (e) {
    const name = project.name;
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Cannot publish ${name}`,
      body: e instanceof Error ? e.message : String(e),
    });
  }
}

/**
 * Run in Container (PLT-5): publishes `project` as a container image with the SDK
 * and runs it with `docker run`, a web project's launch port mapped to the app.
 */
export async function runInContainer(project: SolutionProject): Promise<void> {
  if (!isTauri()) return;
  try {
    const repository = await askImageName(project, tx`Run in Container`, tx`Run`);
    if (!repository) return;
    const cmd = await call(commands.dotnetContainerRun(project.path, repository.trim()));
    const config: RunConfig = {
      id: `container:${project.path}`,
      name: `Container: ${project.name}`,
      kind: "shell",
      command: cmd.command,
      args: cmd.args,
      cwd: cmd.cwd,
      env: { DOTNET_CLI_TELEMETRY_OPTOUT: "1", DOTNET_NOLOGO: "1" },
      detected: true,
      isTest: false,
      debugAdapter: null,
    };
    await executeAction("run.runConfig", config);
  } catch (e) {
    const name = project.name;
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Cannot run ${name} in a container`,
      body: e instanceof Error ? e.message : String(e),
    });
  }
}
