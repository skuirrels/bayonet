import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import type { Variable } from "@/bindings";

const children: Record<number, Variable[]> = {
  5: [
    { name: "Count", value: "3", typeName: "int", variablesReference: 0, evaluateName: null },
    { name: "[0]", value: "{Order}", typeName: "Order", variablesReference: 50, evaluateName: null },
    { name: "[1]", value: "{Order}", typeName: "Order", variablesReference: 51, evaluateName: null },
    { name: "[2]", value: "{Order}", typeName: "Order", variablesReference: 52, evaluateName: null },
  ],
  50: [{ name: "Total", value: "12.5", typeName: "decimal", variablesReference: 0, evaluateName: null }],
  51: [{ name: "Total", value: "3", typeName: "decimal", variablesReference: 0, evaluateName: null }],
  52: [{ name: "Total", value: "40", typeName: "decimal", variablesReference: 0, evaluateName: null }],
  6: [{ name: "[0]", value: "1", typeName: "int", variablesReference: 0, evaluateName: null }],
};

vi.mock("@/platform/ipc", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/platform/ipc")>();
  const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
  return {
    ...real,
    commands: {
      ...real.commands,
      debugVariables: (_s: number, ref: number) => ok(children[ref] ?? []),
      debugEvaluateIn: (_s: number, _frame: number | null, expression: string) =>
        ok({ name: expression, value: "{List}", typeName: null, variablesReference: 6, evaluateName: expression }),
    },
  };
});

import { useDebug, type DebugSession } from "../state";
import { openDataGrid, useOverlays } from "../ui/popoverStore";
import { DataGridDialog } from "./DataGridDialog";

const session: DebugSession = {
  id: 1,
  name: "main",
  adapter: "coreclr",
  parent: null,
  launch: null,
  process: null,
  status: "paused",
  stoppedThread: 1,
  stopReason: "breakpoint",
  stopDescription: null,
  threads: [{ id: 1, name: "Main" }],
  selectedThread: 1,
  frames: [],
  selectedFrame: 7,
  hits: 1,
  exitCode: null,
  caps: null,
};

const totals = () =>
  screen
    .getAllByRole("row")
    .slice(1)
    .map((r) => within(r).getAllByRole("cell")[1]?.textContent);

describe("collection visualiser dialog", () => {
  it("shows the elements as rows and sorts by a column", async () => {
    useDebug.setState({ sessions: { 1: session }, active: 1, stopSeq: 4 });
    render(<DataGridDialog />);
    act(() => openDataGrid(1, 5, "orders", null));
    expect(await screen.findByText("3 elements")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Total" })).toBeTruthy();
    expect(totals()).toEqual(["12.5", "3", "40"]);
    fireEvent.click(screen.getByRole("button", { name: "Total" }));
    expect(totals()).toEqual(["3", "12.5", "40"]);
    fireEvent.click(screen.getByRole("button", { name: "Total" }));
    expect(totals()).toEqual(["40", "12.5", "3"]);
  });

  it("marks the values stale at a later stop when it cannot read them again", async () => {
    useDebug.setState({ sessions: { 1: session }, active: 1, stopSeq: 4 });
    render(<DataGridDialog />);
    act(() => openDataGrid(1, 5, "orders", null));
    await screen.findByText("3 elements");
    act(() => useDebug.setState({ stopSeq: 5 }));
    expect(await screen.findByText("The values are from an earlier stop")).toBeTruthy();
    act(() => useOverlays.setState({ grid: null }));
  });

  it("reads the collection again at a later stop through its expression", async () => {
    useDebug.setState({ sessions: { 1: session }, active: 1, stopSeq: 4 });
    render(<DataGridDialog />);
    act(() => openDataGrid(1, 5, "orders", "orders"));
    await screen.findByText("3 elements");
    act(() => useDebug.setState({ stopSeq: 5 }));
    expect(await screen.findByText("1 element")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Value" })).toBeTruthy();
    act(() => useOverlays.setState({ grid: null }));
  });
});
