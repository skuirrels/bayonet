import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Location, OutlineItem, References, TestNode } from "@/bindings";

/** Open documents: path → text and outline. */
const files = new Map<string, { text: string; outline: OutlineItem[] }>();
/** Reference results by `<path>:<line>`. */
const refs = new Map<string, Location[]>();
const tests = new Map<string, Array<{ project: string; test: TestNode }>>();
const docs: string[] = [];

vi.mock("@/bindings", () => ({
  commands: {
    docSnapshot: async (doc: number) => ({ id: doc, text: files.get(docs[doc]!)!.text }),
    docOutline: async (doc: number) => files.get(docs[doc]!)!.outline,
    lspReferences: async (_r: number, doc: number, pos: { line: number }): Promise<References> => ({
      locations: refs.get(`${docs[doc]}:${pos.line}`) ?? [],
      incomplete: null,
    }),
  },
}));
vi.mock("@tauri-apps/api/core", async (orig) => ({
  ...(await orig<object>()),
  Channel: class {
    onmessage = () => {};
  },
}));
vi.mock("@/platform/ipc", () => ({ call: <T>(p: Promise<T>) => p, isTauri: () => true, nextRequestId: () => 1 }));
vi.mock("@/features/code-insight/ipc", () => ({
  withDocument: async <T>(path: string, fn: (doc: number) => Promise<T>) => {
    docs.push(path);
    return fn(docs.length - 1);
  },
}));
vi.mock("@/editor/api", () => ({ onEditorEvent: () => () => {}, openEditorPaths: () => [] }));
vi.mock("@/features/build/store", () => ({ useBuildRun: { subscribe: () => () => {} } }));
vi.mock("@/platform/project", () => ({ useProject: { getState: () => ({ project: { root: "/s" } }) } }));
vi.mock("./store", () => ({
  runTests: vi.fn(),
  testsIn: (file: string) => tests.get(file) ?? [],
  useUnitTests: { getState: () => ({ busy: null, projects: [] }), subscribe: () => () => {} },
}));

const { affectedTests, changedMembers, memberAt, membersOf } = await import("./continuous");

const pos = (line: number, col = 0) => ({ line, col });
const item = (
  name: string,
  kind: OutlineItem["kind"],
  from: number,
  to: number,
  children: OutlineItem[] = [],
  detail: string | null = null,
): OutlineItem => ({
  name,
  detail,
  kind,
  range: { start: pos(from), end: pos(to, 1) },
  selection_range: { start: pos(from, 4), end: pos(from, 4 + name.length) },
  children,
});
const test = (uid: string, line: number, file: string): TestNode =>
  ({ uid, name: uid, line, file, state: "discovered" }) as unknown as TestNode;
const at = (path: string, line: number): Location => ({ path, range: { start: pos(line, 8), end: pos(line, 12) } });

const GREETER = [
  "class Greeter",
  "{",
  "    public string Greet(string n)",
  "    {",
  '        return "Hi " + n;',
  "    }",
  "    public int Count => 1;",
  "}",
].join("\n");
const GREETER_OUTLINE = [
  item("Greeter", "class", 0, 7, [item("Greet", "method", 2, 5, [], "(string)"), item("Count", "property", 6, 6)]),
];

beforeEach(() => {
  files.clear();
  refs.clear();
  tests.clear();
  docs.length = 0;
});

describe("changed members", () => {
  it("keys members by container and signature, and sees only edited ones", () => {
    const before = membersOf(GREETER, GREETER_OUTLINE);
    expect([...before.keys()]).toEqual(["Greeter.Greet((string))", "Greeter.Count()"]);
    const edited = GREETER.replace('"Hi "', '"Hello, "');
    expect(changedMembers(before, membersOf(edited, GREETER_OUTLINE)).map((m) => m.name)).toEqual(["Greet"]);
    // Trailing spaces are not a change.
    expect(changedMembers(before, membersOf(GREETER.replace("n;", "n;   "), GREETER_OUTLINE))).toEqual([]);
  });

  it("finds the innermost member at a line", () => {
    const members = membersOf(GREETER, GREETER_OUTLINE);
    expect(memberAt(members.values(), 4)?.name).toBe("Greet");
    expect(memberAt(members.values(), 6)?.name).toBe("Count");
    expect(memberAt(members.values(), 0)).toBeUndefined();
  });
});

describe("affected tests", () => {
  const CORE = "/s/src/Core/Greeter.cs";
  const HELPER = "/s/src/Core/Welcome.cs";
  const TESTS = "/s/tests/Core.Tests/GreeterTests.cs";
  const PROJECT = "/s/tests/Core.Tests/Core.Tests.csproj";

  beforeEach(() => {
    files.set(CORE, { text: GREETER, outline: GREETER_OUTLINE });
    files.set(HELPER, {
      text: 'class Welcome\n{\n    public string Say() => new Greeter().Greet("x");\n}',
      outline: [item("Welcome", "class", 0, 3, [item("Say", "method", 2, 2)])],
    });
    files.set(TESTS, {
      text: Array.from({ length: 20 }, () => "").join("\n"),
      outline: [
        item("GreeterTests", "class", 0, 19, [
          item("Greets", "method", 2, 6),
          item("Welcomes", "method", 8, 12),
          item("Unrelated", "method", 14, 18),
        ]),
      ],
    });
    tests.set(TESTS, [
      { project: PROJECT, test: test("t-greets", 3, TESTS) },
      { project: PROJECT, test: test("t-welcomes", 9, TESTS) },
      { project: PROJECT, test: test("t-unrelated", 15, TESTS) },
    ]);
  });

  it("runs the tests that call a changed method, directly or through another method", async () => {
    refs.set(`${CORE}:2`, [at(TESTS, 4), at(HELPER, 2)]);
    refs.set(`${HELPER}:2`, [at(TESTS, 10)]);
    const greet = membersOf(GREETER, GREETER_OUTLINE).get("Greeter.Greet((string))")!;
    const affected = await affectedTests([{ path: CORE, member: greet }]);
    expect(affected.selection).toEqual([{ project: PROJECT, uids: ["t-greets", "t-welcomes"] }]);
    expect(affected.count).toBe(2);
    expect(affected.incomplete).toBe(false);
  });

  it("runs an edited test itself", async () => {
    const members = membersOf(files.get(TESTS)!.text, files.get(TESTS)!.outline);
    const unrelated = [...members.values()].find((m) => m.name === "Unrelated")!;
    const affected = await affectedTests([{ path: TESTS, member: unrelated }]);
    expect(affected.selection).toEqual([{ project: PROJECT, uids: ["t-unrelated"] }]);
  });

  it("finds nothing for a member no test reaches", async () => {
    const count = membersOf(GREETER, GREETER_OUTLINE).get("Greeter.Count()")!;
    expect((await affectedTests([{ path: CORE, member: count }])).count).toBe(0);
  });
});
