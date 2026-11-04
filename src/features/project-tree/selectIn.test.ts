import { afterEach, describe, expect, it } from "vitest";
import { executeAction, getAction, registerActions } from "@/platform/actions";
import { setContext } from "@/platform/context";
import { resolveMenuItems, useContextMenuStore, type MenuItem } from "@/ui";
import { provideEditorService, registerMenuGroup, type EditorService } from "@/workbench/contributions";
import { registerProjectActions, selectInTargets } from "./actions";
import { revealLabel } from "./contextMenu";

const labels = (items: MenuItem[]) => resolveMenuItems(items).map((r) => (r.kind === "separator" ? "-" : r.label));

const disposers: Array<() => void> = [];
afterEach(() => {
  disposers.splice(0).forEach((d) => d());
  useContextMenuStore.setState({ menu: null });
});

describe("Select In (⌥F1)", () => {
  it("lists the project view, contributed targets, Finder and the terminal", () => {
    disposers.push(
      registerMenuGroup({
        id: "test.selectIn",
        menus: ["selectIn"],
        items: () => [{ label: "Structure", run: () => {} }],
      }),
      registerActions([{ id: "terminal.openAt", title: "Open Terminal At", run: () => {} }]),
    );
    expect(labels(selectInTargets("/p/src/a.ts"))).toEqual([
      "Select In",
      "Project View",
      "Structure",
      "-",
      revealLabel(),
      "Terminal",
    ]);
  });

  it("without a path shows the chooser for the active file; with one selects it in the project view", async () => {
    disposers.push(registerProjectActions(), () => setContext("projectOpen", false));
    setContext("projectOpen", true);
    provideEditorService({ activePath: () => "/p/src/a.ts" } as unknown as EditorService);
    expect(getAction("navigate.selectInProjectView")?.title).toBe("Select in Project View");
    await executeAction("navigate.selectIn");
    expect(useContextMenuStore.getState().menu?.items[1]).toMatchObject({ label: "Project View" });
    useContextMenuStore.setState({ menu: null });
    await executeAction("navigate.selectIn", { path: "/p/src/a.ts" });
    expect(useContextMenuStore.getState().menu).toBeNull();
  });
});
