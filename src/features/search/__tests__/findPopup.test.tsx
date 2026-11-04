import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import type { ReplaceRequest } from "../find/replaceFlow";

const replaced: ReplaceRequest[] = [];
const confirms: unknown[] = [];

vi.mock("../find/windowStore", async (orig) => ({
  ...(await orig<typeof import("../find/windowStore")>()),
  ipcFindDeps: () => ({
    validate: async () => {},
    start: async (_id: number, _q: unknown, onFile: (f: unknown) => void) => {
      onFile({
        path: "/p/a.kt",
        relPath: "a.kt",
        matches: [
          { line: 1, text: "val order = 1", ranges: [[4, 9]], textOffset: 0 },
          { line: 4, text: "order()", ranges: [[0, 5]], textOffset: 0 },
        ],
      });
      onFile({
        path: "/p/b.kt",
        relPath: "b.kt",
        matches: [{ line: 0, text: "// order", ranges: [[3, 8]], textOffset: 0 }],
      });
      return null;
    },
    cancel: () => {},
    nextRequestId: () => 1,
  }),
}));
vi.mock("../find/replaceFlow", async (orig) => ({
  ...(await orig<typeof import("../find/replaceFlow")>()),
  startReplace: (r: ReplaceRequest) => {
    replaced.push(r);
    const replacements = r.targets.reduce((n, t) => n + (t.only?.length ?? 0), 0);
    return {
      result: Promise.resolve({ files: r.targets.length, replacements, failed: 0, cancelled: false, elapsedMs: 0 }),
      cancel: () => {},
    };
  },
}));
vi.mock("@/workbench/messages", async (orig) => ({
  ...(await orig<typeof import("@/workbench/messages")>()),
  confirm: async (opts: unknown) => {
    confirms.push(opts);
    return true;
  },
}));

const { FindInFilesOverlay } = await import("../find/FindInFiles");
const { findHistory, hideFindPopup, rememberFindString, runFindPopupCommand, showFindPopup, useFindPopup } =
  await import("../find/store");

const settle = () => act(() => new Promise((r) => setTimeout(r, 250)));
const selectedRows = () =>
  within(screen.getByRole("listbox", { name: "Matches" }))
    .getAllByRole("option")
    .filter((o) => o.getAttribute("aria-selected") === "true");
const field = () => screen.getByLabelText("Text to find");

beforeEach(() => {
  replaced.length = 0;
  confirms.length = 0;
  localStorage.clear();
  useFindPopup.setState({ pinned: false, maskEnabled: false, preserveCase: false, context: "any", replacement: "" });
});

afterEach(() => {
  act(() => hideFindPopup());
});

describe("Find in Files popup", () => {
  it("⇧↓ extends the selection; ↵ in Replace mode replaces the selected rows", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(true, { query: "order" }));
    await settle();
    fireEvent.change(screen.getByLabelText("Replace with"), { target: { value: "item" } });
    fireEvent.keyDown(field(), { key: "ArrowDown", shiftKey: true });
    expect(selectedRows()).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Replace 2 Occurrences" })).toBeEnabled();
    fireEvent.keyDown(field(), { key: "Enter" });
    await settle();
    expect(replaced).toHaveLength(1);
    expect(replaced[0]!.targets).toEqual([
      {
        path: "/p/a.kt",
        only: [
          { line: 1, col: 4 },
          { line: 4, col: 0 },
        ],
      },
    ]);
    // The next row still to replace is selected.
    expect(selectedRows().map((o) => o.textContent)).toEqual([expect.stringContaining("b.kt")]);
  });

  it("⌘-click toggles rows into the selection", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(false, { query: "order" }));
    await settle();
    const options = within(screen.getByRole("listbox", { name: "Matches" })).getAllByRole("option");
    fireEvent.click(options[2]!, { metaKey: true, ctrlKey: true, detail: 1 });
    expect(selectedRows()).toHaveLength(2);
    fireEvent.click(options[1]!, { detail: 1 });
    expect(selectedRows()).toHaveLength(1);
  });

  it("⇧⌥↵ replaces all, asking first only when several files are affected", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(true, { query: "order" }));
    await settle();
    fireEvent.keyDown(field(), { key: "Enter", shiftKey: true, altKey: true });
    await settle();
    expect(confirms).toHaveLength(1);
    expect(replaced[0]!.targets.map((t) => t.path)).toEqual(["/p/a.kt", "/p/b.kt"]);
  });

  it("⌥ mnemonics toggle Preserve Case and the file mask", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(true, { query: "order" }));
    await settle();
    fireEvent.keyDown(field(), { key: "é", code: "KeyE", altKey: true });
    expect(useFindPopup.getState().preserveCase).toBe(true);
    fireEvent.keyDown(field(), { key: "k", code: "KeyK", altKey: true });
    expect(useFindPopup.getState().maskEnabled).toBe(true);
  });

  it("the pin is a remembered toggle that keeps the popup open when a result is opened", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(false, { query: "order" }));
    await settle();
    const pin = screen.getByRole("button", { name: "Pin Window" });
    fireEvent.click(pin);
    expect(pin).toHaveAttribute("aria-pressed", "true");
    expect(localStorage.getItem("openidea.search.findPinned.v1")).toBe("true");
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(useFindPopup.getState().open).toBe(true);
    expect(screen.getByRole("button", { name: "Open in Find Window" })).toBeInTheDocument();
  });

  it("find-next keys move the selection and ⌥↓ lists recent searches", async () => {
    rememberFindString("find", "older");
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(false, { query: "order" }));
    await settle();
    act(() => void runFindPopupCommand("nextResult"));
    act(() => void runFindPopupCommand("nextResult"));
    expect(selectedRows()[0]!.textContent).toContain("b.kt");
    act(() => void runFindPopupCommand("previousResult"));
    expect(selectedRows()[0]!.textContent).toContain("a.kt");
    act(() => void runFindPopupCommand("recentSearches"));
    expect(await screen.findByText("older")).toBeInTheDocument();
    act(() => hideFindPopup());
  });

  it("closing remembers the search text", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(false, { query: "order" }));
    await settle();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    expect(useFindPopup.getState().open).toBe(false);
    expect(findHistory("find")[0]).toBe("order");
  });
});
