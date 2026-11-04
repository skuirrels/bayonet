import { describe, expect, it } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { MethodSignature } from "@/bindings";
import { askSignatureChange, ChangeSignatureHost } from "./ChangeSignatureDialog";

const SIG: MethodSignature = {
  name: "Price",
  kind: "method",
  container: "Shop",
  parameters: [
    { text: "decimal net", name: "net", modifiers: [], hasDefault: false },
    { text: "int count", name: "count", modifiers: [], hasDefault: false },
  ],
  extension: false,
  nameRange: { start: { line: 6, col: 19 }, end: { line: 6, col: 24 } },
  header: "public decimal Price(decimal net, int count)",
};

const refactor = () => screen.getByRole("button", { name: "Refactor" });

describe("Change Signature dialog (CI-6)", () => {
  it("moves and adds parameters, previews them and returns the change", async () => {
    render(<ChangeSignatureHost />);
    let answer: Promise<unknown> = Promise.resolve();
    act(() => {
      answer = askSignatureChange(SIG);
    });
    expect(screen.getByText("Change the signature of method 'Shop.Price'")).toBeTruthy();
    // Nothing changed yet.
    expect((refactor() as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getAllByRole("row")[2]!); // `int count` (after the header row)
    fireEvent.click(screen.getByRole("button", { name: "Move Up" }));
    // A new row goes after the selected one.
    fireEvent.click(screen.getByRole("button", { name: "Add Parameter" }));
    const decls = screen.getAllByLabelText("Parameter declaration") as HTMLInputElement[];
    expect(decls.map((d) => d.value)).toEqual(["int count", "", "decimal net"]);
    fireEvent.change(decls[1]!, { target: { value: "bool round" } });
    // A new parameter without a default needs a value for callers.
    expect(screen.getByText("Give a value for callers to pass for 'round'")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Value for callers", { selector: "input" }), {
      target: { value: "true" },
    });
    expect(screen.getByText("public decimal Price(int count, bool round, decimal net)")).toBeTruthy();
    fireEvent.click(refactor());
    await expect(answer).resolves.toEqual([
      { from: 1, text: "int count", value: null },
      { from: null, text: "bool round", value: "true" },
      { from: 0, text: "decimal net", value: null },
    ]);
    expect(screen.queryByText("Signature preview:")).toBeNull();
  });

  it("resolves null when cancelled", async () => {
    render(<ChangeSignatureHost />);
    let answer: Promise<unknown> = Promise.resolve();
    act(() => {
      answer = askSignatureChange(SIG);
    });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await expect(answer).resolves.toBeNull();
  });
});
