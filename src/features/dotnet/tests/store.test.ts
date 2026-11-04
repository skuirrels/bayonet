import { describe, expect, it, vi } from "vitest";
import type { TestEvent } from "@/bindings";

const attach = vi.fn(async (_pid: number, _name: string, _cwd: string) => {});
const runs: Array<{ run: boolean; coverage: boolean; debug: boolean }> = [];

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (m: unknown) => void = () => {};
  },
}));
vi.mock("@/bindings", () => ({
  commands: {
    dotnetTests: async (
      _r: number,
      run: boolean,
      coverage: boolean,
      debug: boolean,
      _s: unknown,
      channel: { onmessage: (e: TestEvent) => void },
    ) => {
      runs.push({ run, coverage, debug });
      if (debug) channel.onmessage({ kind: "debug", project: "/s/tests/T/T.csproj", pid: 4242 });
    },
    cancelRequest: async () => {},
  },
}));
vi.mock("@/platform/ipc", () => ({
  call: <T>(p: Promise<T>) => p,
  isCancelled: () => false,
  isTauri: () => true,
  nextRequestId: () => 1,
}));
vi.mock("@/platform/project", () => ({ useProject: { getState: () => ({ project: { root: "/s" } }) } }));
vi.mock("../attach", () => ({ attachDotnetDebugger: attach }));
vi.mock("./coverage", () => ({ collectTestCoverage: vi.fn() }));

const { debugTests, runTests } = await import("./store");

describe("debugging tests (DBG-5)", () => {
  it("runs under the debugger and attaches to the test app it reports", async () => {
    await debugTests([{ project: "/s/tests/T/T.csproj", uids: ["a"] }]);
    await vi.waitFor(() => expect(attach).toHaveBeenCalledOnce());
    expect(runs.at(-1)).toEqual({ run: true, coverage: false, debug: true });
    expect(attach.mock.calls[0]!.slice(0, 1)).toEqual([4242]);
    expect(attach.mock.calls[0]![2]).toBe("/s");
  });

  it("does not attach on a plain run", async () => {
    attach.mockClear();
    await runTests();
    expect(runs.at(-1)).toEqual({ run: true, coverage: false, debug: false });
    expect(attach).not.toHaveBeenCalled();
  });
});
