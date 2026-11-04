import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { cyclicExpandWord, expandCandidates, prefixStart } from "./wordCompletion";
import { pythonModulePath, qualifiedReference, rustImplType, rustModulePath } from "./copyReference";

describe("cyclic expand word", () => {
  const text = "alphaOne x alphaTwo y al| alphaThree alphaOne";
  const caret = text.indexOf("|");
  const clean = text.replace("|", "");

  it("finds the prefix", () => {
    expect(prefixStart(clean, caret)).toBe(caret - 2);
    expect(prefixStart("a.b", 2)).toBe(2);
  });

  it("orders nearest first in the search direction, then other editors", () => {
    expect(expandCandidates(clean, caret - 2, caret, "backward", ["alphaZed"])).toEqual([
      "alphaTwo",
      "alphaOne",
      "alphaThree",
      "alphaZed",
    ]);
    expect(expandCandidates(clean, caret - 2, caret, "forward")).toEqual(["alphaThree", "alphaOne", "alphaTwo"]);
    expect(expandCandidates(clean, caret, caret, "backward")).toEqual([]);
  });

  it("cycles through candidates and back to the prefix", () => {
    const state = EditorState.create({ doc: "foo fooBar fooBaz\nfo", selection: { anchor: 20 } });
    const view = new EditorView({ state });
    const word = () => view.state.doc.line(2).text;
    expect(cyclicExpandWord(view, "backward", () => [])).toBeNull();
    expect(word()).toBe("fooBaz");
    cyclicExpandWord(view, "backward", () => []);
    expect(word()).toBe("fooBar");
    cyclicExpandWord(view, "backward", () => []);
    expect(word()).toBe("foo");
    cyclicExpandWord(view, "backward", () => []);
    expect(word()).toBe("fo");
    view.destroy();
  });

  it("reports when nothing matches", () => {
    const view = new EditorView({ state: EditorState.create({ doc: "zzz q", selection: { anchor: 5 } }) });
    expect(cyclicExpandWord(view, "backward", () => [])).toBe("No suitable completions found");
    view.destroy();
  });
});

describe("copy reference", () => {
  const base = { root: "/p", line: 4 };

  it("qualifies Java members with the package", () => {
    const text = "package com.acme.util;\n\nclass Strings {}";
    expect(
      qualifiedReference({ ...base, language: "java", path: "/p/src/Strings.java", text, chain: ["Strings", "trim"] }),
    ).toBe("com.acme.util.Strings#trim");
    expect(
      qualifiedReference({ ...base, language: "java", path: "/p/src/Strings.java", text, chain: ["Strings"] }),
    ).toBe("com.acme.util.Strings");
    expect(
      qualifiedReference({ ...base, language: "java", path: "/p/src/A.java", text: "", chain: ["A", "B", "m"] }),
    ).toBe("A.B#m");
  });

  it("builds Rust paths from the module tree", () => {
    expect(rustModulePath("/p/crates/x/src/lib.rs")).toBe("crate");
    expect(rustModulePath("/p/src/a/b.rs")).toBe("crate::a::b");
    expect(rustModulePath("/p/src/a/mod.rs")).toBe("crate::a");
    expect(rustImplType("impl<T> fmt::Display for Foo<T>")).toBe("Foo");
    expect(rustImplType("impl Bar")).toBe("Bar");
    expect(
      qualifiedReference({
        ...base,
        language: "rust",
        path: "/p/src/net/http.rs",
        text: "",
        chain: ["impl Client", "get"],
      }),
    ).toBe("crate::net::http::Client::get");
  });

  it("uses dotted module paths for Python and package names for Go", () => {
    expect(pythonModulePath("/p/src/pkg/mod.py", "/p")).toBe("pkg.mod");
    expect(pythonModulePath("/p/pkg/__init__.py", "/p")).toBe("pkg");
    expect(
      qualifiedReference({ ...base, language: "python", path: "/p/pkg/mod.py", text: "", chain: ["C", "m"] }),
    ).toBe("pkg.mod.C.m");
    expect(
      qualifiedReference({ ...base, language: "go", path: "/p/x.go", text: "package server\n", chain: ["Run"] }),
    ).toBe("server.Run");
  });

  it("falls back to path:line", () => {
    expect(qualifiedReference({ ...base, language: "java", path: "/p/src/A.java", text: "", chain: [] })).toBe(
      "src/A.java:5",
    );
    expect(qualifiedReference({ ...base, language: "typescript", path: "/p/a.ts", text: "", chain: ["C", "m"] })).toBe(
      "a.ts:C.m",
    );
    expect(
      qualifiedReference({ ...base, root: null, language: "markdown", path: "/x/a.md", text: "", chain: [] }),
    ).toBe("/x/a.md:5");
  });
});
