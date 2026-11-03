import { describe, expect, it } from "vitest";
import type { RunConfig } from "@/platform/ipc";
import { dotnetRunType } from "./runType";

const rc = (args: string[], command = "dotnet") => ({ command, args }) as unknown as RunConfig;

describe(".NET Project run configurations", () => {
  it("round-trips a detected launch profile configuration", () => {
    const detected = rc(["run", "--project", "src/Api/Api.csproj", "--launch-profile", "https", "--", "--seed", "1"]);
    const params = dotnetRunType.paramsFrom!(detected)!;
    expect(params).toEqual({
      project: "src/Api/Api.csproj",
      launchProfile: "https",
      framework: "",
      configuration: "",
      arguments: "--seed 1",
    });
    expect(dotnetRunType.commandLine(params)).toEqual({
      command: "dotnet",
      args: detected.args,
    });
  });

  it("offers detected projects and leaves other commands alone", () => {
    const configs = [rc(["run", "--project", "a.csproj"]), rc(["run", "--project", "a.csproj", "-lp", "x"])];
    expect(dotnetRunType.fields[0]!.suggestions!(configs, {})).toEqual(["a.csproj"]);
    expect(dotnetRunType.paramsFrom!(rc(["test"]))).toBeNull();
    expect(dotnetRunType.paramsFrom!(rc(["run", "--no-build"]))).toBeNull();
    expect(
      dotnetRunType.commandLine({
        project: "a.csproj",
        configuration: "Release",
      })!.args,
    ).toEqual(["run", "--project", "a.csproj", "--configuration", "Release"]);
  });
});
