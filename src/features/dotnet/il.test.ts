import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useEditorBanners, type EditorService } from "@/workbench/contributions";
import { useTasks } from "@/workbench/tasks";
import { isDecompiledSource, registerIlBanner, showIl } from "./il";

const LIST = "/var/folders/x/T/MetadataAsSource/b17c/DecompilationMetadataAsSourceFileProvider/020f/List.cs";
const URI = "dotnet-il://System.Collections.dll/List%601.il?7b7d";

const h = vi.hoisted(() => ({
  uri: vi.fn<(path: string, pos: { line: number; col: number } | null) => Promise<string>>(),
  document: vi.fn<(uri: string) => Promise<unknown>>(),
  opened: [] as string[],
  notes: [] as { level: string; title: string; body?: unknown }[],
  cursor: null as ReturnType<EditorService["activeCursor"]>,
}));

vi.mock("@/bindings", () => ({ commands: { dotnetIlUri: h.uri } }));
vi.mock("@/platform/ipc", async (original) => ({
  ...(await original<typeof import("@/platform/ipc")>()),
  call: <T>(p: Promise<T>) => p,
}));
vi.mock("@/platform/virtualDocs", async (original) => ({
  ...(await original<typeof import("@/platform/virtualDocs")>()),
  virtualDocument: h.document,
}));
vi.mock("@/workbench/contributions", async (original) => ({
  ...(await original<typeof import("@/workbench/contributions")>()),
  maybeEditorService: () => ({
    activeCursor: () => h.cursor,
    openFile: async (path: string) => void h.opened.push(path),
  }),
  notifications: () => ({ notify: (n: (typeof h.notes)[number]) => void h.notes.push(n) }),
}));

beforeEach(() => {
  h.uri.mockReset().mockResolvedValue(URI);
  h.document.mockReset().mockResolvedValue({});
  h.opened.length = 0;
  h.notes.length = 0;
  h.cursor = null;
  useTasks.setState({ tasks: [] });
});

describe("decompiled sources", () => {
  it("are the files of Roslyn's decompilation provider", () => {
    expect(isDecompiledSource(LIST)).toBe(true);
    expect(
      isDecompiledSource(
        "C:\\Users\\me\\AppData\\Local\\Temp\\MetadataAsSource\\b17c\\DecompilationMetadataAsSourceFileProvider\\020f\\List.cs",
      ),
    ).toBe(true);
  });

  it("are not the library's own sources, project files or other languages", () => {
    expect(
      isDecompiledSource("/tmp/MetadataAsSource/b17c/PdbSourceDocumentMetadataAsSourceFileProvider/11aa/Thing.cs"),
    ).toBe(false);
    expect(isDecompiledSource("/p/src/DecompilationMetadataAsSourceFileProvider/List.cs")).toBe(false);
    expect(isDecompiledSource(LIST.replace(/\.cs$/, ".vb"))).toBe(false);
  });
});

describe("Show IL", () => {
  it("opens the IL of the type at the caret of the focused file", async () => {
    h.cursor = { doc: 3, path: "/p/App/Greeter.cs", line: 4, col: 20, offset: 80 };
    await showIl();
    expect(h.uri).toHaveBeenCalledWith("/p/App/Greeter.cs", { line: 4, col: 20 });
    expect(h.document).toHaveBeenCalledWith(URI);
    expect(h.opened).toEqual([URI]);
    expect(h.notes).toEqual([]);
  });

  it("takes the first type of a file whose editor does not have the caret", async () => {
    h.cursor = { doc: 3, path: "/p/App/Greeter.cs", line: 4, col: 20, offset: 80 };
    await showIl(LIST);
    expect(h.uri).toHaveBeenCalledWith(LIST, null);
    expect(h.opened).toEqual([URI]);
  });

  it("does nothing without a file", async () => {
    await showIl();
    expect(h.uri).not.toHaveBeenCalled();
    expect(h.opened).toEqual([]);
  });

  it("says why there is no IL, and opens nothing", async () => {
    h.cursor = { doc: 3, path: "/p/App/Greeter.cs", line: 0, col: 0, offset: 0 };
    h.uri.mockRejectedValue(new Error("App.csproj has not been built: build it to see its IL"));
    await showIl();
    expect(h.opened).toEqual([]);
    expect(h.notes).toEqual([
      expect.objectContaining({
        level: "warning",
        title: "Cannot show IL",
        body: "App.csproj has not been built: build it to see its IL",
      }),
    ]);
  });

  it("reports a failed disassembly itself rather than as a library class", async () => {
    h.document.mockRejectedValue(new Error("the .NET SDK (`dotnet`) was not found"));
    await showIl(LIST);
    expect(h.opened).toEqual([]);
    expect(h.notes.map((n) => n.body)).toEqual(["the .NET SDK (`dotnet`) was not found"]);
  });

  it("shows a background task while it works", async () => {
    let finish!: (uri: string) => void;
    h.uri.mockReturnValue(new Promise<string>((resolve) => (finish = resolve)));
    const shown = showIl(LIST);
    expect(useTasks.getState().tasks).toEqual([
      expect.objectContaining({ title: "Disassembling to IL", message: "List.cs", done: false }),
    ]);
    finish(URI);
    await shown;
    expect(useTasks.getState().tasks.every((t) => t.done)).toBe(true);
  });
});

describe("the banner of decompiled code", () => {
  it("offers Show IL for that file only", async () => {
    const off = registerIlBanner();
    const banner = useEditorBanners.getState().banners.get("dotnet.il")!;
    expect(renderHook(() => banner.useBanner("/p/App/Greeter.cs")).result.current).toBeNull();
    const shown = renderHook(() => banner.useBanner(LIST)).result.current!;
    expect(shown.text).toBe("Decompiled from a referenced assembly.");
    expect(shown.actions!.map((a) => a.label)).toEqual(["Show IL"]);
    shown.actions![0]!.run();
    await vi.waitFor(() => expect(h.opened).toEqual([URI]));
    expect(h.uri).toHaveBeenCalledWith(LIST, null);
    off();
    expect(useEditorBanners.getState().banners.has("dotnet.il")).toBe(false);
  });
});
