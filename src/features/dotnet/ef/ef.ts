/**
 * Entity Framework Core (PLT-2): migrations, database updates, SQL scripts and
 * DbContext scaffolding through `dotnet ef`, run in the Run tool window.
 */
import { commands } from "@/bindings";
import type { EfOperation } from "@/bindings";
import { notifyMissingTool } from "@/features/coverage/executor";
import { executeAction, type Action } from "@/platform/actions";
import { tx } from "@/platform/i18n";
import { call, type RunConfig } from "@/platform/ipc";
import { chooseItem, promptInput } from "@/workbench/inputDialogs";
import { notifications } from "@/workbench/contributions";
import { useSolution, type SolutionProject } from "../projects/store";

/** Providers offered when scaffolding (the package names `dotnet ef dbcontext scaffold` takes). */
export const EF_PROVIDERS = [
  "Microsoft.EntityFrameworkCore.SqlServer",
  "Microsoft.EntityFrameworkCore.Sqlite",
  "Npgsql.EntityFrameworkCore.PostgreSQL",
  "Pomelo.EntityFrameworkCore.MySql",
  "Oracle.EntityFrameworkCore",
  "Microsoft.EntityFrameworkCore.Cosmos",
];

/** Latest and revert-all entries of the Update Database list. */
export const LATEST = "(latest migration)";
export const REVERT_ALL = "0 (revert every migration)";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** A C# identifier, as `dotnet ef migrations add` needs for the class name. */
export function migrationNameError(name: string): string | null {
  if (!name.trim()) return tx`Enter a migration name`;
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name.trim()) ? null : tx`A migration name is a C# identifier`;
}

/** The Update Database choices: the latest, every migration newest first, then reverting all. */
export function updateTargets(migrations: readonly string[]): string[] {
  return [LATEST, ...[...migrations].reverse(), REVERT_ALL];
}

/** The `migration` argument of an Update Database choice. */
export function updateMigration(choice: string): string | null {
  if (choice === LATEST) return null;
  if (choice === REVERT_ALL) return "0";
  return choice;
}

export function isEfProject(project: SolutionProject | null): boolean {
  return !!project && useSolution.getState().efProjects.includes(project.path);
}

/** Loads which projects use EF Core (from their restore output). */
export async function loadEfProjects(): Promise<void> {
  try {
    const efProjects = await call(commands.dotnetEfProjects());
    useSolution.setState({ efProjects });
  } catch {
    useSolution.setState({ efProjects: [] });
  }
}

const TITLES: Record<EfOperation["kind"], string> = {
  addMigration: "Add Migration",
  removeMigration: "Remove Last Migration",
  updateDatabase: "Update Database",
  script: "Generate SQL Script",
  scaffoldDbContext: "Scaffold DbContext",
};

/** Runs `operation` on `project` in the Run tool window. */
export async function runEf(project: SolutionProject, operation: EfOperation): Promise<void> {
  try {
    const cmd = await call(commands.dotnetEfCommand(project.path, operation));
    if (cmd.status === "missingTool") {
      notifyMissingTool("dotnet", cmd);
      return;
    }
    const config: RunConfig = {
      id: `ef:${operation.kind}:${project.path}`,
      name: `EF Core: ${TITLES[operation.kind]} (${project.name})`,
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
    notifications().notify({
      group: "dotnet",
      level: "error",
      title: tx`Cannot run dotnet ef`,
      body: errorText(e),
    });
  }
}

export async function addMigration(project: SolutionProject): Promise<void> {
  const name = await promptInput({
    title: tx`Add Migration`,
    label: tx`Name:`,
    placeholder: "InitialCreate",
    okLabel: tx`Add`,
    validate: migrationNameError,
  });
  if (name) await runEf(project, { kind: "addMigration", name, outputDir: null });
}

export async function updateDatabase(project: SolutionProject): Promise<void> {
  const migrations = await call(commands.dotnetEfMigrations(project.path)).catch(() => [] as string[]);
  const choice = await chooseItem({ title: tx`Update Database to`, items: updateTargets(migrations) });
  if (choice) await runEf(project, { kind: "updateDatabase", migration: updateMigration(choice) });
}

export async function generateScript(project: SolutionProject): Promise<void> {
  const output = await promptInput({
    title: tx`Generate SQL Script`,
    label: tx`Output file:`,
    initial: `${project.dir}/migrations.sql`,
    okLabel: tx`Generate`,
  });
  if (output) await runEf(project, { kind: "script", output });
}

export async function scaffoldDbContext(project: SolutionProject): Promise<void> {
  const connection = await promptInput({
    title: tx`Scaffold DbContext`,
    label: tx`Connection string:`,
    placeholder: "Data Source=app.db",
    okLabel: tx`Next`,
  });
  if (!connection) return;
  const provider = await chooseItem({ title: tx`Database provider`, items: EF_PROVIDERS });
  if (!provider) return;
  const outputDir = await promptInput({
    title: tx`Scaffold DbContext`,
    label: tx`Output folder:`,
    initial: "Models",
    okLabel: tx`Scaffold`,
    validate: () => null,
  });
  if (outputDir === null) return;
  await runEf(project, {
    kind: "scaffoldDbContext",
    connection,
    provider,
    outputDir: outputDir || null,
    context: null,
  });
}

const projectOf = (arg: unknown): SolutionProject | null => {
  const dir = (arg as { dir?: unknown } | undefined)?.dir;
  return typeof dir === "string" ? (useSolution.getState().projects.find((p) => p.dir === dir) ?? null) : null;
};

/** The EF Core actions (project context menu › Entity Framework Core); `arg.dir` is the project folder. */
export function efActions(): Action[] {
  const action = (id: string, title: string, run: (p: SolutionProject) => Promise<void>, icon?: string): Action => ({
    id: `dotnet.ef.${id}`,
    title,
    category: "Entity Framework Core",
    hidden: true,
    when: "projectOpen",
    ...(icon ? { icon } : {}),
    run: ({ arg }) => {
      const project = projectOf(arg);
      if (project) void run(project);
    },
  });
  return [
    action("addMigration", "Add Migration…", addMigration, "plus"),
    action("removeMigration", "Remove Last Migration", (p) => runEf(p, { kind: "removeMigration" })),
    action("updateDatabase", "Update Database…", updateDatabase),
    action("script", "Generate SQL Script…", generateScript),
    action("scaffold", "Scaffold DbContext…", scaffoldDbContext),
  ];
}
