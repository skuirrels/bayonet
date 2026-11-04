import { describe, expect, it } from "vitest";
import { Text } from "@codemirror/state";
import type { Diagnostic } from "@/bindings";
import { diagnosticsFor, setFileDiagnostics, subscribeDiagnostics, toLintDiagnostics } from "./diagnostics";

const d = (
  line: number,
  col: number,
  endCol: number,
  severity: Diagnostic["severity"] = "error",
  over: Partial<Diagnostic> = {},
): Diagnostic => ({
  range: { start: { line, col }, end: { line, col: endCol } },
  severity,
  message: `m${line}:${col}`,
  source: null,
  code: null,
  ...over,
});

describe("editor diagnostics store", () => {
  it("keeps one list per source and notifies subscribers of the path", () => {
    let calls = 0;
    const off = subscribeDiagnostics("/p/x.rs", () => calls++);
    setFileDiagnostics("/p/x.rs", "rust-analyzer", [d(0, 0, 1)]);
    setFileDiagnostics("/p/x.rs", "cargo", [d(1, 0, 1, "warning")]);
    expect(diagnosticsFor("/p/x.rs").map((x) => x.source)).toEqual(["rust-analyzer", "cargo"]);
    setFileDiagnostics("/p/x.rs", "rust-analyzer", []);
    expect(diagnosticsFor("/p/x.rs").map((x) => x.source)).toEqual(["cargo"]);
    setFileDiagnostics("/p/other.rs", "cargo", [d(0, 0, 1)]);
    expect(calls).toBe(3);
    // Clearing an unknown source is a no-op.
    setFileDiagnostics("/p/x.rs", "unknown", []);
    expect(calls).toBe(3);
    off();
  });
});

describe("toLintDiagnostics", () => {
  const doc = Text.of(["fn main() {", "  let x = 1;", "}"]);

  it("converts ranges, sorts and adds mark classes", () => {
    const out = toLintDiagnostics(doc, [
      d(1, 6, 7, "warning", { unnecessary: true, source: "rustc", code: "unused" }),
      d(0, 3, 7, "error", { deprecated: true }),
    ]);
    expect(out).toEqual([
      { from: 3, to: 7, severity: "error", message: "m0:3", source: undefined, markClass: "cm-lsp-deprecated" },
      {
        from: 18,
        to: 19,
        severity: "warning",
        message: "m1:6",
        source: "rustc unused",
        markClass: "cm-lsp-unnecessary",
      },
    ]);
  });

  it("widens empty ranges to one character", () => {
    expect(toLintDiagnostics(doc, [d(0, 2, 2)])[0]).toMatchObject({ from: 2, to: 3 });
    // At a line end the previous character is used.
    expect(toLintDiagnostics(doc, [d(2, 1, 1)])[0]).toMatchObject({ from: doc.length - 1, to: doc.length });
  });
});
