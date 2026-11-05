import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { PreviewEvent } from "@/bindings";

const h = vi.hoisted(() => ({
  channels: [] as { onmessage: (e: unknown) => void }[],
  started: [] as { path: string; text: string; scale: number }[],
  updates: [] as string[],
  cancelled: [] as number[],
  opened: [] as unknown[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (e: unknown) => void = () => undefined;
    constructor() {
      h.channels.push(this);
    }
  },
}));

vi.mock("@/platform/ipc", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/platform/ipc")>();
  return { ...real, isTauri: () => true };
});

vi.mock("@/bindings", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/bindings")>();
  const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
  return {
    ...real,
    commands: {
      ...real.commands,
      dotnetXamlPreview: (_id: number, path: string, text: string, scale: number) => {
        h.started.push({ path, text, scale });
        return new Promise(() => undefined);
      },
      dotnetXamlPreviewUpdate: (_id: number, text: string) => {
        h.updates.push(text);
        return ok(true);
      },
      cancelRequest: (id: number) => {
        h.cancelled.push(id);
        return Promise.resolve(null);
      },
    },
  };
});

vi.mock("@/workbench/contributions", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/workbench/contributions")>();
  return {
    ...real,
    editorService: () => ({ openFile: (path: string, o: unknown) => (h.opened.push({ path, o }), Promise.resolve()) }),
  };
});

import { useLiveXaml } from "./live";
import { reduce } from "./session";
import { setXamlMode } from "./modes";
import { XamlPreviewEditor } from "./XamlPreviewEditor";

const PATH = "/p/App/MainWindow.axaml";
const send = (e: PreviewEvent) => act(() => h.channels.at(-1)!.onmessage(e));

describe("Avalonia previewer (CI-16)", () => {
  it("keeps the last good frame while the XAML has an error", () => {
    const frame: PreviewEvent = { kind: "frame", png: "AAA", width: 800, height: 480, scale: 2 };
    let s = reduce({ phase: "idle", message: null, frame: null, error: null }, frame);
    s = reduce(s, { kind: "xamlError", message: "Unable to resolve type Buton", line: 15, column: 5 });
    expect(s.frame?.src).toBe("data:image/png;base64,AAA");
    expect(s.error?.line).toBe(15);
    expect(reduce(s, { kind: "xamlLoaded" }).error).toBeNull();
  });

  it("starts with the editor's text, sends edits, shows frames and errors, and stops when hidden", async () => {
    setXamlMode(PATH, "split");
    useLiveXaml.setState({ [PATH]: "<Window/>" });
    render(
      <XamlPreviewEditor path={PATH} tabId="t" actionsInTabs>
        <div>editor</div>
      </XamlPreviewEditor>,
    );
    expect(await screen.findByText("Building the project…")).toBeTruthy();
    expect(h.started.at(-1)).toMatchObject({ path: PATH, text: "<Window/>" });

    send({ kind: "phase", phase: "ready", message: null });
    send({ kind: "frame", png: "AAA", width: 800, height: 480, scale: 2 });
    const img = screen.getByAltText("Preview of the XAML") as HTMLImageElement;
    expect([img.width, img.height]).toEqual([400, 240]);
    fireEvent.click(screen.getByRole("button", { name: "Zoom In" }));
    expect(img.width).toBe(600);

    act(() => useLiveXaml.setState({ [PATH]: "<Window><Buton/></Window>" }));
    expect(h.updates.at(-1)).toBe("<Window><Buton/></Window>");
    send({ kind: "xamlError", message: "Unable to resolve type Buton", line: 0, column: 8 });
    expect(screen.getByRole("alert").textContent).toContain("Unable to resolve type Buton");
    fireEvent.click(screen.getByRole("button", { name: "Line 1" }));
    expect(h.opened.at(-1)).toMatchObject({ path: PATH, o: { selection: { line: 0, col: 8 } } });

    const before = h.cancelled.length;
    act(() => setXamlMode(PATH, "editor"));
    expect(h.cancelled.length).toBe(before + 1);
    expect(screen.queryByAltText("Preview of the XAML")).toBeNull();
  });

  it("shows why the preview failed", async () => {
    setXamlMode(PATH, "preview");
    useLiveXaml.setState({ [PATH]: "<Window/>" });
    render(
      <XamlPreviewEditor path={PATH} tabId="t" actionsInTabs>
        <div>editor</div>
      </XamlPreviewEditor>,
    );
    await screen.findByText("Building the project…");
    send({ kind: "phase", phase: "failed", message: "the project does not reference Avalonia (no previewer host)" });
    expect(screen.getByRole("status").textContent).toContain("does not reference Avalonia");
    const starts = h.started.length;
    fireEvent.click(screen.getByRole("button", { name: "Build and Restart the Preview" }));
    expect(h.started.length).toBe(starts + 1);
  });
});
