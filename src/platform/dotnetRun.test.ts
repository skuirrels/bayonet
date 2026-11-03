import { describe, expect, it } from "vitest";
import { dotnetRunArgs, parseDotnetRun } from "./dotnetRun";

describe("dotnet run command lines", () => {
  it("reads the project, profile, framework and program arguments", () => {
    const run = parseDotnetRun("/usr/local/share/dotnet/dotnet", [
      "run",
      "--project",
      "src/App/App.csproj",
      "-lp",
      "https",
      "--framework=net10.0",
      "--no-build",
      "--",
      "--seed",
      "1",
    ]);
    expect(run).toEqual({
      project: "src/App/App.csproj",
      launchProfile: "https",
      framework: "net10.0",
      configuration: "",
      programArgs: ["--seed", "1"],
      options: ["--no-build"],
    });
    expect(dotnetRunArgs(run!)).toEqual([
      "run",
      "--project",
      "src/App/App.csproj",
      "--launch-profile",
      "https",
      "--framework",
      "net10.0",
      "--no-build",
      "--",
      "--seed",
      "1",
    ]);
  });

  it("is null for other commands", () => {
    expect(parseDotnetRun("dotnet", ["test"])).toBeNull();
    expect(parseDotnetRun("cargo", ["run"])).toBeNull();
    expect(parseDotnetRun("dotnet.exe", ["run"])).not.toBeNull();
  });
});
