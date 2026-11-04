import { doubleShift, expect, test } from "./fixtures";

test("double Shift opens Search Everywhere with its tabs; Escape closes it", async ({ workbench: page }) => {
  await doubleShift(page);
  const dialog = page.getByRole("dialog", { name: "Search Everywhere" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("combobox")).toBeFocused();
  for (const tab of ["All", "Classes", "Files", "Symbols", "Actions", "Text"]) {
    await expect(dialog.getByRole("tab", { name: tab })).toBeVisible();
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("Search Everywhere finds and runs an action", async ({ workbench: page }) => {
  await doubleShift(page);
  const dialog = page.getByRole("dialog", { name: "Search Everywhere" });
  await dialog.getByRole("combobox").fill("Language Servers");
  const option = dialog.getByRole("option", { name: /Language Servers/ }).first();
  await expect(option).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("list", { name: /language servers/i })).toBeVisible();
});

test("Search Everywhere finds a file and opens it", async ({ workbench: page }) => {
  // TODO(search): plain-browser mode has no in-memory `findFiles`, so the Files
  // source returns nothing. Un-fixme once search/everywhere gets browser deps
  // (e.g. backed by the project-tree sample file system).
  test.fixme(true, "Search Everywhere has no in-memory file source in browser mode");
  await doubleShift(page);
  const dialog = page.getByRole("dialog", { name: "Search Everywhere" });
  await dialog.getByRole("combobox").fill("OrderService");
  await expect(dialog.getByRole("option", { name: /OrderService\.kt/ })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tab", { name: "OrderService.kt" })).toBeVisible();
});
