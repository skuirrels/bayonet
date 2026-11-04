/**
 * Attaching netcoredbg to a .NET process the IDE knows about: a debug run's
 * test app (DBG-5), which waits for it, or a running Aspire resource.
 */
import { tx } from "@/platform/i18n";

export async function attachDotnetDebugger(pid: number, name: string, cwd: string): Promise<void> {
  const [{ startSession }, { workflowSettings }] = await Promise.all([
    import("@/features/debugger/state"),
    import("@/platform/workflowSettings"),
  ]);
  await startSession(
    {
      name: tx`${name} (${pid})`,
      adapter: "netcoredbg",
      request: "attach",
      program: null,
      args: [],
      cwd,
      env: {},
      extraJson: JSON.stringify({ processId: pid, justMyCode: workflowSettings().debugger.skipLibraryCode }),
    },
    { rerunnable: false },
  );
}
