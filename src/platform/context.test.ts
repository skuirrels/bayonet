import { describe, expect, it } from "vitest";
import { evaluateWhen } from "./context";

describe("when clauses", () => {
  const ctx = { editorFocus: true, lang: "rust", count: 2, debug: false };
  it.each([
    ["", true],
    ["editorFocus", true],
    ["!editorFocus", false],
    ["editorFocus && debug", false],
    ["editorFocus || debug", true],
    ["lang == rust", true],
    ["lang == 'rust'", true],
    ["lang != rust", false],
    ["count == 2", true],
    ["!(debug || !editorFocus)", true],
    ["missing", false],
  ])("%s → %s", (expr, expected) => {
    expect(evaluateWhen(expr, ctx)).toBe(expected);
  });

  it("rejects malformed clauses", () => {
    expect(() => evaluateWhen("a &&", ctx)).toThrow();
    expect(() => evaluateWhen("(a", ctx)).toThrow();
  });
});
