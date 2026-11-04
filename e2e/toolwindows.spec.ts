import { expect, test, visibleToolWindows } from "./fixtures";

test("⌘1 focuses, hides and re-shows the Project tool window", async ({ workbench: page }) => {
  const project = page.getByRole("region", { name: "Project" });
  await page.keyboard.press("Meta+1");
  await expect(page.getByRole("tree", { name: "Project" })).toBeFocused();
  await page.keyboard.press("Meta+1");
  await expect(project).toBeHidden();
  await page.keyboard.press("Meta+1");
  await expect(project).toBeVisible();
});

test("⌘4 toggles Run; Shift+Escape hides the active tool window", async ({ workbench: page }) => {
  const run = page.getByRole("region", { name: "Run" });
  await page.keyboard.press("Meta+4");
  await expect(run).toBeVisible();
  await page.keyboard.press("Meta+4");
  await expect(run).toBeHidden();
  await page.keyboard.press("Meta+4");
  await expect(run).toBeVisible();
  await page.keyboard.press("Shift+Escape");
  await expect(run).toBeHidden();
});

test("⌥F12 opens the Terminal tool window and toggles it closed", async ({ workbench: page }) => {
  const terminal = page.getByRole("region", { name: "Terminal" });
  await page.keyboard.press("Alt+F12");
  await expect(terminal).toBeVisible();
  expect(await visibleToolWindows(page)).toContain("Terminal");
  await page.keyboard.press("Alt+F12");
  await expect(terminal).toBeHidden();
});

test("⌘⇧F12 hides all tool windows and restores them", async ({ workbench: page }) => {
  await page.keyboard.press("Meta+6");
  await expect(page.getByRole("region", { name: "Problems" })).toBeVisible();
  await page.keyboard.press("Meta+Shift+F12");
  expect(await visibleToolWindows(page)).toEqual([]);
  await page.keyboard.press("Meta+Shift+F12");
  await expect(page.getByRole("region", { name: "Project" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Problems" })).toBeVisible();
});
