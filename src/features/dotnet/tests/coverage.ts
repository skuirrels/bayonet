/**
 * Coverage from the Unit Tests window (TST-4): Microsoft.Testing.Platform test
 * apps write Cobertura reports through the Microsoft.Testing.Extensions.CodeCoverage
 * extension, which become one suite in the Coverage tool window.
 */
import { commands } from "@/bindings";
import { addSuite } from "@/features/coverage/store";
import { formatPercent } from "@/features/coverage/model";
import { tx } from "@/platform/i18n";
import { call } from "@/platform/ipc";
import { useProject } from "@/platform/project";
import { notifications, toolWindowService } from "@/workbench/contributions";

/** The MTP extension that collects coverage (the MSTest metapackage brings it in). */
export const COVERAGE_PACKAGE = "Microsoft.Testing.Extensions.CodeCoverage";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Test projects of a run that wrote no report: they lack the coverage extension (or wrote nothing). */
export function projectsWithoutReport(
  reports: ReadonlyMap<string, string>,
  ran: ReadonlyArray<{ path: string; name: string }>,
): Array<{ path: string; name: string }> {
  return ran.filter((p) => !reports.has(p.path));
}

async function addPackage(projects: string[]) {
  try {
    await call(commands.nugetInstall(projects, COVERAGE_PACKAGE, null));
    notifications().notify({
      group: "tests",
      level: "info",
      key: "tests-coverage-package",
      title: tx`Added ${COVERAGE_PACKAGE}`,
      body: tx`Run the tests with coverage again to collect it.`,
    });
  } catch (e) {
    notifications().notify({
      group: "tests",
      level: "error",
      key: "tests-coverage-package",
      title: tx`Cannot add ${COVERAGE_PACKAGE}`,
      body: errorText(e),
    });
  }
}

/** Turns the reports of a coverage run into a suite and offers the extension to projects that had none. */
export async function collectTestCoverage(
  reports: ReadonlyMap<string, string>,
  ran: ReadonlyArray<{ path: string; name: string }>,
): Promise<void> {
  const missing = projectsWithoutReport(reports, ran);
  const count = missing.length;
  const project = missing[0]?.name ?? "";
  if (count)
    notifications().notify({
      group: "tests",
      level: "warning",
      key: "tests-coverage-missing",
      title: count === 1 ? tx`No coverage data for ${project}` : tx`No coverage data for ${count} test projects`,
      body: tx`Coverage needs the ${COVERAGE_PACKAGE} package in the test project.`,
      actions: [{ label: tx`Add Package`, run: () => void addPackage(missing.map((p) => p.path)) }],
    });
  if (reports.size === 0) return;
  const root = useProject.getState().project?.root ?? "";
  try {
    const data = await call(
      commands.coverageCollect({
        kind: "cobertura",
        runner: "Microsoft Code Coverage",
        paths: [...reports.values()],
        base: root,
      }),
    );
    addSuite(tx`Unit Tests`, data);
    try {
      toolWindowService().show("coverage");
    } catch {
      /* no workbench (tests) */
    }
    const [covered, total] = data.files.reduce((a, f) => [a[0]! + f.linesCovered, a[1]! + f.linesTotal], [0, 0]);
    const percent = formatPercent(covered!, total!);
    notifications().notify({
      group: "tests",
      level: "info",
      key: "tests-coverage-done",
      title: tx`Coverage of the unit tests: ${percent} lines`,
    });
  } catch (e) {
    notifications().notify({
      group: "tests",
      level: "error",
      key: "tests-coverage-done",
      title: tx`Cannot read the coverage reports`,
      body: errorText(e),
    });
  }
}
