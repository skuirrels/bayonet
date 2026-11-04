import { expect, PROJECT, test } from "./fixtures";

test("renders title bar, stripes, tool window and status bar", async ({ workbench: page }) => {
  const title = page.locator("header").first();
  await expect(title.getByRole("button", { name: PROJECT })).toBeVisible();
  await expect(title.getByRole("button", { name: "Search Everywhere" })).toBeVisible();

  const left = page.getByRole("toolbar", { name: "Tool windows" });
  for (const name of ["Project", "Commit", "Git", "Terminal", "Problems"]) {
    await expect(left.getByRole("button", { name, exact: true })).toBeVisible();
  }
  await expect(page.getByRole("toolbar", { name: "Right tool windows" })).toBeVisible();

  await expect(page.getByRole("region", { name: "Project" })).toBeVisible();
  const status = page.getByRole("contentinfo", { name: "Status bar" });
  await expect(status).toBeVisible();
  // Empty editor area shows the IDEA-style hints.
  await expect(page.getByText("Search Everywhere", { exact: true }).last()).toBeVisible();
});

test("stripe buttons toggle tool windows", async ({ workbench: page }) => {
  const stripe = page.getByRole("toolbar", { name: "Tool windows" });
  const problems = page.getByRole("region", { name: "Problems" });
  await expect(problems).toBeHidden();
  await stripe.getByRole("button", { name: "Problems", exact: true }).click();
  await expect(problems).toBeVisible();
  await stripe.getByRole("button", { name: "Problems", exact: true }).click();
  await expect(problems).toBeHidden();
});
