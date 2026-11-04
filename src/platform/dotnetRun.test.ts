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
      watch: false,
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

  it("reads and writes Hot Reload runs through dotnet watch", () => {
    const args = dotnetRunArgs({ watch: true, project: "src/Api/Api.csproj", launchProfile: "http" });
    expect(args).toEqual([
      "watch",
      "--non-interactive",
      "run",
      "--project",
      "src/Api/Api.csproj",
      "--launch-profile",
      "http",
    ]);
    expect(parseDotnetRun("dotnet", args)).toMatchObject({
      watch: true,
      project: "src/Api/Api.csproj",
      launchProfile: "http",
      options: [],
    });
    expect(parseDotnetRun("dotnet", ["watch", "--project", "A.csproj"])).toMatchObject({
      watch: true,
      project: "A.csproj",
    });
    expect(parseDotnetRun("dotnet", ["build"])).toBeNull();
  });
});
