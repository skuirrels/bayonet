import { describe, expect, it, vi } from "vitest";
import { CompletionContext, type CompletionResult, type CompletionSource } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import type { BindableMember, BindableType } from "@/bindings";

const member = (name: string, typeName: string | null, line = 3): BindableMember => ({
  name,
  typeName,
  kind: typeName ? "property" : "command",
  path: "/s/ViewModels/MainViewModel.cs",
  range: { start: { line, col: 4 }, end: { line, col: 4 + name.length } },
  declaredIn: "MainViewModel",
});
const TYPES: Record<string, BindableType> = {
  MainViewModel: {
    name: "MainViewModel",
    namespace: "App.ViewModels",
    path: "/s/ViewModels/MainViewModel.cs",
    range: { start: { line: 1, col: 13 }, end: { line: 1, col: 26 } },
    members: [
      member("Owner", "Models.User?"),
      member("Items", "ObservableCollection<Item>"),
      member("SaveCommand", null, 9),
    ],
  },
  User: {
    name: "User",
    namespace: "Models",
    path: "/s/Models/User.cs",
    range: { start: { line: 2, col: 13 }, end: { line: 2, col: 17 } },
    members: [{ ...member("Name", "string", 4), path: "/s/Models/User.cs", declaredIn: "User" }],
  },
  Item: {
    name: "Item",
    namespace: "App.Models",
    path: "/s/Models/Item.cs",
    range: { start: { line: 0, col: 0 }, end: { line: 0, col: 4 } },
    members: [{ ...member("Title", "string"), path: "/s/Models/Item.cs", declaredIn: "Item" }],
  },
};
const asked: Array<[string, string | null]> = [];
const opened: Array<[string, number]> = [];

vi.mock("@/bindings", () => ({
  commands: {
    csharpTypeMembers: async (name: string, ns: string | null) => {
      asked.push([name, ns]);
      return TYPES[name] ?? null;
    },
  },
}));
vi.mock("@/platform/ipc", () => ({ call: <T>(p: Promise<T>) => p, isTauri: () => true }));
vi.mock("@/workbench/contributions", () => ({
  editorService: () => ({
    openFile: async (path: string, o: { selection: { line: number } }) => void opened.push([path, o.selection.line]),
  }),
}));
vi.mock("@/editor/lsp/hint", () => ({ showHint: () => {} }));

const { followXaml, xamlEditor } = await import("./editor");

const XAML = `<Window xmlns:vm="using:App.ViewModels" x:DataType="vm:MainViewModel">
  <TextBlock Text="{Binding Owner.}" />
  <ListBox ItemsSource="{Binding Items[0].Ti}" Tag="{Binding }" />
</Window>`;

async function complete(text: string, pos: number): Promise<string[]> {
  const state = EditorState.create({
    doc: text,
    extensions: xamlEditor({
      doc: 1,
      path: "/s/Views/MainWindow.axaml",
      language: "xml",
      uiMode: "XML",
      readOnly: false,
      virtual: false,
    } as never),
  });
  const [source] = state.languageDataAt<CompletionSource>("openideaCompletion", pos);
  const r = (await source!(new CompletionContext(state, pos, true))) as CompletionResult | null;
  return r?.options.map((o) => o.label) ?? [];
}

describe("XAML editor (CI-16)", () => {
  it("completes binding paths through members and collection elements", async () => {
    expect(await complete(XAML, XAML.indexOf("{Binding }") + 9)).toEqual(["Owner", "Items", "SaveCommand"]);
    expect(await complete(XAML, XAML.indexOf("Owner.") + 6)).toEqual(["Name"]);
    expect(asked).toContainEqual(["User", "Models"]);
    expect(await complete(XAML, XAML.indexOf("].Ti") + 4)).toEqual(["Title"]);
    expect(asked).toContainEqual(["Item", null]);
  });

  it("goes to the member or type under the caret", async () => {
    expect(await followXaml(XAML, XAML.indexOf("Items[0]") + 2)).toBe(true);
    expect(opened.at(-1)).toEqual(["/s/ViewModels/MainViewModel.cs", 3]);
    expect(await followXaml(XAML, XAML.indexOf('MainViewModel"') + 4)).toBe(true);
    expect(opened.at(-1)).toEqual(["/s/ViewModels/MainViewModel.cs", 1]);
    expect(await followXaml(XAML, XAML.indexOf("Missing"))).toBe(false);
  });
});
