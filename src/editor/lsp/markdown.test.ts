import { describe, expect, it, vi } from "vitest";

// A fresh module per test: the parser is loaded lazily and cached module-wide.
async function freshMarkdown() {
  vi.resetModules();
  return import("./markdown");
}

describe("markdown rendering", () => {
  it("shows plain text until the parser loads, then the rendered markup", async () => {
    const md = await freshMarkdown();
    const el = md.renderMarkup({ kind: "markdown", value: "**bold** <img src=x onerror=alert(1)>" });
    expect(el.querySelector(".cm-lsp-plain")?.textContent).toBe("**bold** <img src=x onerror=alert(1)>");
    await md.loadMarkdown();
    await vi.waitFor(() => expect(el.querySelector("strong")?.textContent).toBe("bold"));
    expect(el.querySelector("img")?.getAttribute("onerror")).toBeNull();
  });

  it("renders synchronously once loaded", async () => {
    const md = await freshMarkdown();
    await md.loadMarkdown();
    expect(md.markupToHtml({ kind: "markdown", value: "`x`" })).toBe("<p><code>x</code></p>\n");
    expect(md.markupToHtml({ kind: "plaintext", value: "<b>" })).toBe('<p class="cm-lsp-plain">&#60;b&#62;</p>');
  });
});
