import { describe, expect, it } from "vitest";
import { relativeDir } from "./project";

describe("relativeDir", () => {
  it("is empty for files directly in the project root", () => {
    expect(relativeDir("/p/app", "/p/app/README.md")).toBe("");
  });
  it("is project-relative for nested files", () => {
    expect(relativeDir("/p/app", "/p/app/src/lib/a.ts")).toBe("src/lib");
  });
  it("keeps the absolute directory outside the project", () => {
    expect(relativeDir("/p/app", "/other/x.ts")).toBe("/other");
  });
});
