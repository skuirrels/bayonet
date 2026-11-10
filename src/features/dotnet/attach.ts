/**
 * Attaching netcoredbg to a .NET process the IDE knows about: a debug run's
 * test app (DBG-5), which waits for it, or a running Aspire resource. The
 * debugger feature does it (`debug.attachDotnet`) and reports its errors.
 */
import { executeAction } from "@/platform/actions";
import { tx } from "@/platform/i18n";

export async function attachDotnetDebugger(pid: number, name: string, cwd: string): Promise<void> {
  if (!(await executeAction("debug.attachDotnet", { pid, name, cwd })))
    throw new Error(tx("Cannot attach the debugger"));
}
