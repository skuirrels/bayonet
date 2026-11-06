import { describe, expect, it } from "vitest";
import { visualStudioMacos } from "./visualstudio-macos";
import { visualStudioAdditions, visualStudioOverrides, visualStudioWindows } from "./visualstudio-windows";

const holders = (km: Record<string, string[]>, binding: string) =>
  Object.entries(km)
    .filter(([, keys]) => keys.includes(binding))
    .map(([id]) => id)
    .sort();

// The default keymap of this port; the e2e specs run on the IDEA preset instead.
describe("Visual Studio keymaps", () => {
  it("bind every Visual Studio shortcut to its action", () => {
    for (const [id, keys] of Object.entries(visualStudioOverrides)) {
      for (const key of keys) expect(visualStudioWindows[id], `${id} ${key}`).toContain(key);
    }
  });

  it("take a Visual Studio key from the IDEA actions that had it", () => {
    // Additions share on purpose: Alt+Down moves the line, or opens a search field's history.
    const intended = { ...visualStudioAdditions };
    for (const [id, keys] of Object.entries(visualStudioOverrides)) intended[id] = [...(intended[id] ?? []), ...keys];
    for (const keys of Object.values(visualStudioOverrides)) {
      for (const key of keys) expect(holders(visualStudioWindows, key), key).toEqual(holders(intended, key));
    }
  });

  it("read Ctrl as ⌘ on macOS and leave the system's keys alone", () => {
    expect(visualStudioMacos["navigate.gotoFile"]).toContain("Meta+Shift+T");
    expect(visualStudioMacos["navigate.declaration"]).toEqual(["F12"]);
    expect(visualStudioMacos["find.usages"]).toContain("Shift+F12");
    const system = /^Meta\+(Q|H|M|Tab|Shift\+Tab|`|Space)$/;
    for (const [id, keys] of Object.entries(visualStudioMacos)) {
      for (const key of keys) expect(system.test(key.split(" ")[0]!), `${id} ${key}`).toBe(false);
    }
  });
});
