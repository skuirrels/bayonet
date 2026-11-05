import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useEditorBanners } from "@/workbench/contributions";
import { indexLanguages, installDegradedBanner, languageOfPath, useFileLanguages } from "./degradedBanner";
import { upsertServer, useServers } from "./servers";

const restarted = vi.hoisted((): number[] => []);
vi.mock("@/platform/ipc", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/platform/ipc")>();
  return {
    ...real,
    commands: {
      ...real.commands,
      lspRestart: (id: number) => (restarted.push(id), Promise.resolve({ status: "ok" as const, data: null })),
    },
  };
});

const LANGUAGES = [
  { id: "csharp", extensions: ["cs", "csx"], filenames: [] },
  { id: "typescript", extensions: ["ts", "d.ts"], filenames: [] },
  { id: "msbuild", extensions: ["csproj", "props"], filenames: ["Directory.Build.props"] },
];
const roslyn = { id: 7 as never, name: "roslyn-language-server", languages: ["csharp"] };

afterEach(() => {
  useServers.setState({ servers: new Map() });
  useFileLanguages.setState({ byName: new Map() });
});

describe("Degraded code insight banner (NFR-4)", () => {
  it("finds a file's language by name or extension", () => {
    indexLanguages(LANGUAGES);
    const { byName } = useFileLanguages.getState();
    expect(languageOfPath("/p/App/Program.cs", byName)).toBe("csharp");
    expect(languageOfPath("/p/types/index.d.ts", byName)).toBe("typescript");
    expect(languageOfPath("/p/Directory.Build.props", byName)).toBe("msbuild");
    expect(languageOfPath("/p/README", byName)).toBeNull();
  });

  it("shows while the server is crashed or restarting, and goes once it runs", async () => {
    indexLanguages(LANGUAGES);
    const off = installDegradedBanner();
    const banner = useEditorBanners.getState().banners.get("code-insight.degraded")!;
    const { result } = renderHook(() => banner.useBanner("/p/App/Program.cs"));
    act(() => upsertServer({ ...roslyn, status: "ready" }));
    expect(result.current).toBeNull();

    act(() => upsertServer({ ...roslyn, status: "crashed", message: "exited with code 137\nRestarting…" }));
    expect(result.current?.level).toBe("warning");
    expect(result.current?.text).toMatch(/^roslyn-language-server crashed\..*editing and saving still work/);
    result.current!.actions![0]!.run();
    await vi.waitFor(() => expect(restarted).toEqual([7]));

    act(() => upsertServer({ ...roslyn, status: "starting" }));
    expect(result.current?.text).toMatch(/is restarting after a crash/);
    act(() => upsertServer({ ...roslyn, status: "ready" }));
    expect(result.current).toBeNull();

    // A server that is merely starting (not after a crash) is not degraded.
    act(() => upsertServer({ ...roslyn, status: "starting" }));
    expect(result.current).toBeNull();
    // Other languages' files are not affected.
    act(() => upsertServer({ ...roslyn, status: "crashed" }));
    expect(renderHook(() => banner.useBanner("/p/web/index.ts")).result.current).toBeNull();
    off();
  });

  it("stays quiet while another server for the language runs", () => {
    indexLanguages(LANGUAGES);
    const off = installDegradedBanner();
    const banner = useEditorBanners.getState().banners.get("code-insight.degraded")!;
    act(() => {
      upsertServer({ ...roslyn, status: "crashed" });
      upsertServer({ id: 8 as never, name: "csharp-ls", languages: ["csharp"], status: "ready" });
    });
    expect(renderHook(() => banner.useBanner("/p/App/Program.cs")).result.current).toBeNull();
    off();
  });
});
