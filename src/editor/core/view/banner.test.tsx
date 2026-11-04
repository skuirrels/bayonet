import { describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { registerEditorBanner } from "@/workbench/contributions";
import { useDocMeta, type DocMeta } from "../model/document";
import { documentBanner, EditorBanners } from "./Banner";

describe("editor banners", () => {
  it("shows the document notice through the banner extension point, before contributed ones", () => {
    const off = [
      registerEditorBanner(documentBanner),
      registerEditorBanner({ id: "test.other", useBanner: () => ({ text: "Other notice" }) }),
    ];
    act(() => useDocMeta.setState({ "/p/a.bin": { banner: "binary" } as DocMeta }));
    render(<EditorBanners path="/p/a.bin" />);
    const notices = screen.getAllByRole("status").map((n) => n.textContent);
    expect(notices).toEqual(["This file appears to be binary and is shown read-only.", "Other notice"]);
    act(() => useDocMeta.setState({ "/p/a.bin": { banner: null } as DocMeta }));
    expect(screen.getAllByRole("status").map((n) => n.textContent)).toEqual(["Other notice"]);
    off.forEach((f) => f());
  });
});
