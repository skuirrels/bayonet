import { describe, expect, it, vi } from "vitest";
import { solutionItems } from "./chooseSolution";

describe("solutionItems", () => {
  it("lists each solution with its folder, the current one marked", () => {
    const pick = vi.fn();
    const items = solutionItems(
      "/repos",
      ["/repos/All.slnx", "/repos/Api/Api.sln", "/repos/Web/src/Web.slnx"],
      "/repos/Api/Api.sln",
      pick,
    );
    expect(items.map((i) => [i.label, i.detail, i.icon])).toEqual([
      ["All.slnx", undefined, "module"],
      ["Api.sln", "Api", "check"],
      ["Web.slnx", "Web/src", "module"],
    ]);
    void items[2]!.run();
    expect(pick).toHaveBeenCalledWith("/repos/Web/src/Web.slnx");
  });
});
