import { expect, test } from "@playwright/test";

for (const theme of ["light", "dark"] as const) {
  for (const extension of ["http", "rest"] as const) {
    test(`highlights request lines and JSON bodies in ${extension} files (${theme})`, async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(`/e2e/httpHighlight.html?theme=${theme}${extension === "rest" ? "&rest" : ""}`);
      const token = (text: string) =>
        page
          .locator("#http .cm-line span")
          .filter({ hasText: new RegExp(`^${text}$`) })
          .first();
      const keyword = theme === "light" ? "rgb(15, 84, 214)" : "rgb(108, 149, 235)";
      const property = theme === "light" ? "rgb(0, 147, 161)" : "rgb(102, 195, 204)";
      const string = theme === "light" ? "rgb(140, 108, 65)" : "rgb(201, 162, 109)";
      for (const method of ["GRPC", "WEBSOCKET", "POST"]) await expect(token(method)).toHaveCSS("color", keyword);
      await expect(token('"name"')).toHaveCSS("color", property);
      await expect(token('"World"')).toHaveCSS("color", string);
      await expect(token("12")).toHaveCSS("color", theme === "light" ? "rgb(171, 47, 107)" : "rgb(237, 148, 192)");
      for (const literal of ["true", "false", "null"]) await expect(token(literal)).toHaveCSS("color", keyword);
      await expect(token("Content-Type")).toHaveCSS("color", property);
      await expect(page.locator("#json .cm-line").filter({ hasText: '"name"' }).first()).toBeVisible();
      for (const text of ['"name"', '"World"', "12", "true", "false", "null", "{", "[", ":", ","]) {
        const color = async (editor: string) =>
          page.locator(`${editor} .cm-line`).evaluateAll((lines, target) => {
            for (const line of lines) {
              if (!/^\s*["{[]/.test(line.textContent ?? "")) continue;
              const index = line.textContent?.indexOf(target) ?? -1;
              if (index < 0) continue;
              const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
              let offset = 0;
              while (walker.nextNode()) {
                const node = walker.currentNode;
                if (offset + (node.textContent?.length ?? 0) > index)
                  return getComputedStyle(node.parentElement!).color;
                offset += node.textContent?.length ?? 0;
              }
            }
            return null;
          }, text);
        const jsonColor = await color("#json");
        expect(jsonColor, `JSON color for ${text}`).not.toBeNull();
        expect(await color("#http"), `HTTP color for ${text}`).toBe(jsonColor);
      }
      expect(errors).toEqual([]);
    });
  }
}
