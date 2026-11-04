import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { registerAction } from "@/platform/actions";
import type { SeDeps } from "../everywhere/controller";

const fakeDeps = (): SeDeps => ({
  findFiles: async (q) =>
    q ? [{ path: "/p/src/OrderController.kt", relPath: "src/OrderController.kt", score: 1, indices: [4, 5, 6] }] : [],
  findSymbols: async () => [],
  searchText: async () => {},
  cancel: () => {},
  nextRequestId: () => 1,
  actions: () => [...(globalThis as unknown as { __actions: () => [] }).__actions()],
  isEnabled: () => true,
  recentFiles: async () => ["/p/README.md"],
  recentActions: () => [],
  root: () => "/p",
  now: () => 0,
});

vi.mock("../everywhere/deps", () => ({
  ipcDeps: () => fakeDeps(),
  loadLanguages: async () => [
    { id: "kotlin", name: "Kotlin", extensions: ["kt"] },
    { id: "markdown", name: "Markdown", extensions: ["md"] },
  ],
}));
vi.mock("../find/windowStore", async (orig) => ({
  ...(await orig<typeof import("../find/windowStore")>()),
  ipcFindDeps: () => ({
    validate: async () => {},
    start: async (_id: number, _q: unknown, onFile: (f: unknown) => void) => {
      onFile({
        path: "/p/a.kt",
        relPath: "a.kt",
        matches: [{ line: 2, text: "val order = 1", ranges: [[4, 9]], textOffset: 0 }],
      });
      return null;
    },
    cancel: () => {},
    nextRequestId: () => 1,
  }),
}));

const { allActions } = await import("@/platform/actions");
(globalThis as unknown as { __actions: typeof allActions }).__actions = allActions;
const { SearchEverywhereOverlay } = await import("../everywhere/SearchEverywhere");
const { showSearchEverywhere, hideSearchEverywhere, useSearchEverywhere, pushSearchHistory, searchHistory } =
  await import("../everywhere/store");
const { FindInFilesOverlay } = await import("../find/FindInFiles");
const { showFindPopup, hideFindPopup } = await import("../find/store");

const tick = () => act(() => new Promise((r) => setTimeout(r, 100)));

afterEach(() => {
  act(() => {
    hideSearchEverywhere();
    hideFindPopup();
  });
});

describe("Search Everywhere dialog", () => {
  it("shows recent files for an empty query, then flat results; Enter runs an action", async () => {
    const run = vi.fn();
    const dispose = registerAction({ id: "test.order", title: "Order Pizza", category: "Test", run });
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("all"));
    await tick();
    expect(screen.getByText("Recent Files")).toBeInTheDocument();
    expect(within(document.getElementById("se-results")!).getByText("README.md")).toBeInTheDocument();

    const input = screen.getByLabelText("Search query");
    fireEvent.change(input, { target: { value: "ord" } });
    await tick();
    const results = within(document.getElementById("se-results")!);
    expect(results.queryByText("Files")).toBeNull();
    // All mixes sources by relevance: the shorter prefix hit (the action) leads.
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(screen.getByText("src/OrderController.kt")).toBeInTheDocument();
    fireEvent.keyDown(input, { key: "ArrowUp" });
    fireEvent.keyDown(input, { key: "Enter" });
    await tick();
    expect(useSearchEverywhere.getState().open).toBe(false);
    expect(run).toHaveBeenCalled();
    dispose();
  });

  it("the preview toggle is remembered", async () => {
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("all"));
    await tick();
    const toggle = screen.getByRole("button", { name: "Preview" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    expect(useSearchEverywhere.getState().preview).toBe(true);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(localStorage.getItem("openidea.search.everywherePreview.v1")).toBe("true");
    fireEvent.click(toggle);
    expect(useSearchEverywhere.getState().preview).toBe(false);
  });

  it("/ lists scope commands; picking one or typing '/files q' switches the tab", async () => {
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("all", { query: "" }));
    await tick();
    const input = screen.getByLabelText("Search query");
    fireEvent.change(input, { target: { value: "/fi" } });
    await tick();
    expect(within(document.getElementById("se-results")!).getByText("/files")).toBeInTheDocument();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(useSearchEverywhere.getState()).toMatchObject({ tab: "files", query: "", open: true });

    fireEvent.change(input, { target: { value: "/text order" } });
    expect(useSearchEverywhere.getState()).toMatchObject({ tab: "text", query: "order" });
  });

  it("⌥↑ / ⌥↓ browse the tab's search history and restore the draft", async () => {
    localStorage.removeItem("openidea.search.everywhereHistory.v1");
    pushSearchHistory("all", "older");
    pushSearchHistory("all", "newer");
    pushSearchHistory("files", "other tab");
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("all", { query: "draft" }));
    await tick();
    const input = screen.getByLabelText("Search query");
    const key = (k: string) => fireEvent.keyDown(input, { key: k, altKey: true });
    key("ArrowUp");
    expect(useSearchEverywhere.getState().query).toBe("newer");
    key("ArrowUp");
    key("ArrowUp");
    expect(useSearchEverywhere.getState().query).toBe("older");
    key("ArrowDown");
    key("ArrowDown");
    expect(useSearchEverywhere.getState().query).toBe("draft");
    expect(searchHistory("all")).toEqual(["newer", "older"]);
  });

  it("⇥ cycles scopes and repeating the scope action toggles non-project items", async () => {
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("all"));
    await tick();
    const input = screen.getByLabelText("Search query");
    fireEvent.keyDown(input, { key: "Tab" });
    expect(useSearchEverywhere.getState().tab).toBe("classes");
    fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
    fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
    expect(useSearchEverywhere.getState().tab).toBe("text");
    // Double Shift on another tab switches to All, keeping the query; on All it toggles the scope.
    fireEvent.change(input, { target: { value: "kept" } });
    act(() => showSearchEverywhere("all", { query: "selection" }));
    expect(useSearchEverywhere.getState()).toMatchObject({ tab: "all", query: "kept", includeNonProject: false });
    act(() => showSearchEverywhere("all"));
    expect(useSearchEverywhere.getState().includeNonProject).toBe(true);
  });

  it("labels the Actions tab's checkbox and assigns a shortcut with ⌥↵", async () => {
    const assign = vi.fn();
    const disposers = [
      registerAction({ id: "test.pizza", title: "Order Pizza", category: "Test", run: () => {} }),
      registerAction({ id: "settings.assignShortcut", title: "Assign", hidden: true, run: ({ arg }) => assign(arg) }),
    ];
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("actions", { query: "order pizza" }));
    await tick();
    expect(screen.getByLabelText("Include disabled actions")).toBeInTheDocument();
    expect(screen.queryByLabelText("Include non-project items")).toBeNull();
    fireEvent.keyDown(screen.getByLabelText("Search query"), { key: "Enter", altKey: true });
    await tick();
    expect(assign).toHaveBeenCalledWith("test.pizza");
    expect(useSearchEverywhere.getState().open).toBe(false);
    disposers.forEach((d) => d());
  });

  it("filters Files by file type from the funnel, remembering the choice", async () => {
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("files", { query: "order" }));
    await tick();
    expect(document.getElementById("se-results")!.textContent).toContain("OrderController");
    fireEvent.click(screen.getByRole("button", { name: "Filter" }));
    await tick();
    fireEvent.click(screen.getByLabelText("Kotlin"));
    await tick();
    expect(useSearchEverywhere.getState().filters.files.languages).toEqual(["kotlin"]);
    expect(JSON.parse(localStorage.getItem("openidea.search.everywhereFilters.v1")!).files.languages).toEqual([
      "kotlin",
    ]);
    expect(document.getElementById("se-results")!.textContent).not.toContain("OrderController");
    act(() =>
      useSearchEverywhere.setState({
        filters: { ...useSearchEverywhere.getState().filters, files: { languages: [] } },
      }),
    );
  });

  it("searches everywhere when nothing is found in the project", async () => {
    render(<SearchEverywhereOverlay />);
    act(() => showSearchEverywhere("symbols", { query: "zzz" }));
    await tick();
    expect(useSearchEverywhere.getState()).toMatchObject({ includeNonProject: true, notFound: "zzz" });
    // A query that no longer contains the not-found one narrows back to the project.
    fireEvent.change(screen.getByLabelText("Search query"), { target: { value: "other" } });
    expect(useSearchEverywhere.getState()).toMatchObject({ includeNonProject: false, notFound: null });
  });
});

describe("Find in Files dialog", () => {
  it("streams results and previews the replacement", async () => {
    render(<FindInFilesOverlay />);
    act(() => showFindPopup(true, { query: "order" }));
    await act(() => new Promise((r) => setTimeout(r, 250)));
    expect(screen.getByText("1 match in 1 file")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Replace with"), { target: { value: "item" } });
    expect(screen.getByText("item", { ignore: "script, style, textarea" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Replace All" })).toBeEnabled();
  });
});
