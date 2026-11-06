import { describe, expect, it } from "vitest";
import { convertIcls } from "../intellijTheme";

describe("convertIcls", () => {
  it("maps Rider's identifier kinds and doc comment tags to their tokens", () => {
    const { tokens } = convertIcls(`<scheme name="R" version="142" parent_scheme="Darcula"><colors /><attributes>
      <option name="DEFAULT_DOC_COMMENT_TAG"><value><option name="FOREGROUND" value="487D34" /></value></option>
      <option name="ReSharper.DELEGATE_IDENTIFIER"><value><option name="FOREGROUND" value="E1BFFF" /></value></option>
      <option name="ReSharper.EVENT_IDENTIFIER"><value><option name="FOREGROUND" value="ED94C0" /></value></option>
      <option name="ReSharper.NAMESPACE_IDENTIFIER"><value><option name="FOREGROUND" value="C191FF" /></value></option>
    </attributes></scheme>`);
    expect(tokens["--syn-doc-comment-tag"]?.toLowerCase()).toBe("#487d34");
    expect(tokens["--syn-struct"]?.toLowerCase()).toBe("#e1bfff");
    expect(tokens["--syn-event"]?.toLowerCase()).toBe("#ed94c0");
    expect(tokens["--syn-namespace"]?.toLowerCase()).toBe("#c191ff");
  });
});
