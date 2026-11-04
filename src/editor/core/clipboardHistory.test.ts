// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import {
  MAX_CLIPBOARD_ENTRIES,
  clearClipboardHistory,
  clipLabel,
  clipboardHistory,
  copiedText,
  installClipboardTracking,
  recordClipboard,
} from "./clipboardHistory";

beforeEach(() => clearClipboardHistory());

describe("clipboard history (Paste from History)", () => {
  it("keeps entries newest first, without duplicates, capped", () => {
    recordClipboard("a");
    recordClipboard("b");
    recordClipboard("a");
    recordClipboard("");
    expect(clipboardHistory()).toEqual(["a", "b"]);
    for (let i = 0; i < MAX_CLIPBOARD_ENTRIES + 5; i++) recordClipboard(`x${i}`);
    expect(clipboardHistory()).toHaveLength(MAX_CLIPBOARD_ENTRIES);
    expect(clipboardHistory()[0]).toBe(`x${MAX_CLIPBOARD_ENTRIES + 4}`);
  });

  it("copies the selections, or whole caret lines without a selection", () => {
    const doc = "one\ntwo\nthree";
    const ext = EditorState.allowMultipleSelections.of(true);
    const sel = EditorState.create({
      doc,
      extensions: ext,
      selection: EditorSelection.create([EditorSelection.range(0, 3), EditorSelection.range(8, 13)]),
    });
    expect(copiedText(sel)).toBe("one\nthree");
    const carets = EditorState.create({
      doc,
      extensions: ext,
      selection: EditorSelection.create([
        EditorSelection.cursor(1),
        EditorSelection.cursor(2),
        EditorSelection.cursor(5),
      ]),
    });
    expect(copiedText(carets)).toBe("one\ntwo\n");
  });

  it("labels rows by their first non-blank line and line count", () => {
    expect(clipLabel("\n  foo()\nbar\n")).toEqual({ label: "foo()", detail: "3 lines" });
    expect(clipLabel("x".repeat(100), 10).label).toBe(`${"x".repeat(9)}…`);
    expect(clipLabel("   ").label).toBe("(whitespace)");
  });

  describe("outside editors", () => {
    let dispose: () => void;
    afterEach(() => dispose());
    it("records the selected text of inputs", () => {
      dispose = installClipboardTracking();
      const input = document.createElement("input");
      input.value = "hello world";
      document.body.appendChild(input);
      input.setSelectionRange(6, 11);
      input.dispatchEvent(new Event("copy", { bubbles: true }));
      expect(clipboardHistory()[0]).toBe("world");
      input.remove();
    });
  });
});
