/** Loading the process list of the Attach to Process popup for the chosen target. */
import { call, commands } from "@/platform/ipc";
import { LOCAL_TARGET, sshProcesses } from "./dotnetSsh";
import { useOverlays } from "./ui/popoverStore";

let latest = 0;

/**
 * Lists the processes of `target` in the popup: this machine's, or the .NET processes on
 * the host of an SSH configuration (connecting first). A later call, or closing the
 * popup, drops the result.
 */
export async function loadAttachProcesses(target: string = LOCAL_TARGET) {
  const load = ++latest;
  const current = () => load === latest && useOverlays.getState().attach != null;
  useOverlays.setState({ attach: { processes: null, error: null, target } });
  try {
    const processes =
      target === LOCAL_TARGET ? await call(commands.debugAttachableProcesses()) : await sshProcesses(target);
    if (current()) useOverlays.setState({ attach: { processes, error: null, target } });
  } catch (e) {
    const error = (e instanceof Error ? e.message : String(e)) || "Cannot list processes";
    if (current()) useOverlays.setState({ attach: { processes: [], error, target } });
  }
}
