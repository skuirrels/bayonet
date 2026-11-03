import { beforeEach, describe, expect, it, vi } from "vitest";

const lspProjectContexts = vi.fn();
const lspSetProjectContext = vi.fn();
vi.mock("@/platform/ipc", () => ({
  isTauri: () => true,
  call: async (p: Promise<unknown>) => p,
  commands: { lspProjectContexts, lspSetProjectContext },
}));

const { chooseContext, frameworkOf, hasChoice, loadContexts, useFrameworks } = await import("./frameworks");

const net10 = {
  id: "a|Core.csproj ($net10.0)",
  label: "Fixture.Core (net10.0)",
  kind: 2,
};
const ns20 = {
  id: "b|Core.csproj ($netstandard2.0)",
  label: "Fixture.Core (netstandard2.0)",
  kind: 2,
};

describe("target framework switcher", () => {
  beforeEach(() => {
    lspProjectContexts.mockReset();
    lspSetProjectContext.mockReset().mockResolvedValue(null);
    useFrameworks.setState({
      doc: null,
      path: null,
      contexts: null,
      chosen: new Map(),
    });
  });

  it("names the framework of a context label", () => {
    expect(frameworkOf("Fixture.Core (net10.0)")).toBe("net10.0");
    expect(frameworkOf("Miscellaneous Files")).toBe("Miscellaneous Files");
    expect(hasChoice({ contexts: [net10], active: 0 })).toBe(false);
    expect(hasChoice({ contexts: [net10, ns20], active: 0 })).toBe(true);
  });

  it("switches the document and re-applies the choice when the file opens again", async () => {
    lspProjectContexts.mockResolvedValue({
      contexts: [net10, ns20],
      active: 0,
    });
    await loadContexts(7, "/w/Greeter.cs");
    expect(useFrameworks.getState().contexts?.active).toBe(0);
    await chooseContext(ns20);
    expect(lspSetProjectContext).toHaveBeenCalledWith(7, ns20);
    expect(useFrameworks.getState().contexts?.active).toBe(1);

    lspSetProjectContext.mockClear();
    await loadContexts(9, "/w/Greeter.cs");
    expect(lspSetProjectContext).toHaveBeenCalledWith(9, ns20);
    expect(useFrameworks.getState().contexts?.active).toBe(1);
  });

  it("shows nothing for other files or when the server has no contexts", async () => {
    await loadContexts(null, null);
    expect(lspProjectContexts).not.toHaveBeenCalled();
    lspProjectContexts.mockRejectedValue(new Error("unsupported"));
    await loadContexts(3, "/w/a.cs");
    expect(useFrameworks.getState().contexts).toBeNull();
  });
});
