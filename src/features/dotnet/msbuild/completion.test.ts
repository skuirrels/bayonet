import { describe, expect, it } from "vitest";
import { definedNames, enclosingElement, msbuildCompletions } from "./completion";

const PROJECT = [
  '<Project Sdk="Microsoft.NET.Sdk">',
  "  <PropertyGroup>",
  "    <TargetFramework>net10.0</TargetFramework>",
  "    <MyFlag>true</MyFlag>",
  "  </PropertyGroup>",
  "  <ItemGroup>",
  '    <Widget Include="a" />',
  "  </ItemGroup>",
  '  <Target Name="Stamp" />',
  "</Project>",
].join("\n");

/** Completions with `|` marking the caret, inserted at `marker` in PROJECT. */
function at(marker: string, typed: string) {
  const i = PROJECT.indexOf(marker) + marker.length;
  const text = PROJECT.slice(0, i) + typed + PROJECT.slice(i);
  const r = msbuildCompletions(text, i + typed.length);
  return r && { from: r.from - i, labels: r.options.map((o) => o.label) };
}

describe("MSBuild completion", () => {
  it("finds the enclosing element", () => {
    expect(enclosingElement("<Project><PropertyGroup><A>1</A>")).toBe("propertygroup");
    expect(enclosingElement('<Project><ItemGroup><B Include="x" /></ItemGroup>')).toBe("project");
    expect(enclosingElement("<Project><!-- <Target> -->")).toBe("project");
  });

  it("collects what the file defines", () => {
    expect(definedNames(PROJECT)).toEqual({
      properties: ["TargetFramework", "MyFlag"],
      items: ["Widget"],
      targets: ["Stamp"],
    });
  });

  it("offers properties in a PropertyGroup and items in an ItemGroup", () => {
    const props = at("<PropertyGroup>\n", "    <Nul");
    expect(props?.from).toBe("    <".length);
    expect(props?.labels).toContain("Nullable");
    expect(props?.labels).toContain("MyFlag");
    expect(props?.labels).not.toContain("PackageReference");
    const items = at("<ItemGroup>\n", "    <");
    expect(items?.labels).toContain("PackageReference");
    expect(items?.labels).toContain("Widget");
    expect(at('<Project Sdk="Microsoft.NET.Sdk">\n', "  <")?.labels).toContain("PropertyGroup");
    expect(at('<Target Name="Stamp"', "><")?.labels).toContain("Exec");
  });

  it("offers properties, items and targets in references", () => {
    expect(at("<MyFlag>", "$(Tar")?.labels).toEqual(
      expect.arrayContaining(["TargetFramework", "MSBuildProjectDirectory"]),
    );
    expect(at("<MyFlag>", "@(")?.labels).toContain("Widget");
    const targets = at('<Target Name="Stamp"', ' AfterTargets="Build;Co');
    expect(targets?.from).toBe(' AfterTargets="Build;'.length);
    expect(targets?.labels).toEqual(expect.arrayContaining(["CoreCompile", "Stamp"]));
  });

  it("offers nothing in text content", () => {
    expect(at("<TargetFramework>", "net")).toBeNull();
  });
});
