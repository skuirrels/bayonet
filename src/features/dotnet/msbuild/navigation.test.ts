import { describe, expect, it } from "vitest";
import {
  ancestors,
  findPackageVersion,
  findPropertyDefinition,
  msbuildTargetAt,
  resolveMsbuildPath,
} from "./navigation";

const at = (text: string, marker: string) => text.indexOf(marker) + 1;

describe("msbuildTargetAt", () => {
  const csproj = [
    "<Project>",
    "  <PropertyGroup><OutputPath>$(ArtifactsRoot)/bin</OutputPath></PropertyGroup>",
    '  <ItemGroup><PackageReference Include="xunit.v3" /></ItemGroup>',
    '  <ItemGroup><ProjectReference Include="..\\Core\\Core.csproj" /></ItemGroup>',
    '  <Import Project="$(MSBuildThisFileDirectory)shared.props" />',
    "</Project>",
  ].join("\n");

  it("finds a property use", () => {
    expect(msbuildTargetAt(csproj, at(csproj, "ArtifactsRoot"))).toEqual({ kind: "property", name: "ArtifactsRoot" });
  });

  it("finds a package on its id or its element name", () => {
    expect(msbuildTargetAt(csproj, at(csproj, "unit.v3"))).toEqual({ kind: "package", id: "xunit.v3" });
    expect(msbuildTargetAt(csproj, at(csproj, "PackageReference"))).toEqual({ kind: "package", id: "xunit.v3" });
  });

  it("finds a referenced or imported file", () => {
    expect(msbuildTargetAt(csproj, at(csproj, "Core\\Core"))).toEqual({ kind: "path", path: "..\\Core\\Core.csproj" });
    expect(msbuildTargetAt(csproj, at(csproj, "shared.props"))).toEqual({
      kind: "path",
      path: "$(MSBuildThisFileDirectory)shared.props",
    });
  });

  it("has nothing elsewhere", () => {
    expect(msbuildTargetAt(csproj, at(csproj, "PropertyGroup"))).toBeNull();
    expect(msbuildTargetAt(csproj, 2)).toBeNull();
  });
});

describe("definitions", () => {
  const props = [
    "<Project>",
    "  <!-- <Nullable>disable</Nullable> -->",
    "  <PropertyGroup>",
    '    <Nullable Condition="true">enable</Nullable>',
    "  </PropertyGroup>",
    "  <ItemGroup>",
    '    <PackageVersion Include="NUnit" Version="5.0.0" />',
    "  </ItemGroup>",
    "</Project>",
  ].join("\n");

  it("finds a central package version, ignoring case", () => {
    expect(findPackageVersion(props, "nunit")).toEqual({ line: 6, col: 4 });
    expect(findPackageVersion(props, "MSTest")).toBeNull();
  });

  it("finds a property definition outside comments", () => {
    expect(findPropertyDefinition(props, "Nullable")).toEqual({ line: 3, col: 4 });
    expect(findPropertyDefinition(props, "Null")).toBeNull();
  });
});

describe("paths", () => {
  it("resolves relative, Windows-style and MSBuild-relative paths", () => {
    expect(resolveMsbuildPath("/s/src/App", "..\\Core\\Core.csproj")).toBe("/s/src/Core/Core.csproj");
    expect(resolveMsbuildPath("/s", "$(MSBuildThisFileDirectory)build/x.props")).toBe("/s/build/x.props");
    expect(resolveMsbuildPath("/s", "$(Other)/x.props")).toBeNull();
    expect(resolveMsbuildPath("C:/s/App", "..\\..\\..\\x.props")).toBe("C:/x.props");
  });

  it("lists a directory and its ancestors, nearest first", () => {
    expect(ancestors("/a/b")).toEqual(["/a/b", "/a"]);
  });
});
