/**
 * Completion in MSBuild files (CI-18): element names by their parent (properties
 * in a `PropertyGroup`, items in an `ItemGroup`, tasks in a `Target`), `$(…)`
 * properties, `@(…)` item types and target names in `DependsOnTargets` and its
 * kin. Well-known names come with the ones the file itself defines.
 */
import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import { EditorSelection, EditorState, type Extension } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import type { EditorDocContext } from "@/editor/api";
import { isVirtualUri } from "@/platform/virtualDocs";
import { MSBUILD_FILE } from "./navigation";

/** Common properties of SDK-style projects. */
export const PROPERTIES = [
  "AllowUnsafeBlocks",
  "AnalysisLevel",
  "AnalysisMode",
  "AssemblyName",
  "AssemblyVersion",
  "Authors",
  "BaseIntermediateOutputPath",
  "BaseOutputPath",
  "CentralPackageTransitivePinningEnabled",
  "CheckForOverflowUnderflow",
  "Company",
  "Configuration",
  "Copyright",
  "DebugType",
  "DefineConstants",
  "Deterministic",
  "Description",
  "EnableDefaultCompileItems",
  "EnableNETAnalyzers",
  "EnforceCodeStyleInBuild",
  "FileVersion",
  "GenerateAssemblyInfo",
  "GenerateDocumentationFile",
  "GeneratePackageOnBuild",
  "ImplicitUsings",
  "InformationalVersion",
  "IntermediateOutputPath",
  "InvariantGlobalization",
  "IsPackable",
  "IsPublishable",
  "IsTestProject",
  "LangVersion",
  "ManagePackageVersionsCentrally",
  "NoWarn",
  "Nullable",
  "OutputPath",
  "OutputType",
  "PackageId",
  "PackageLicenseExpression",
  "PackageProjectUrl",
  "PackageReadmeFile",
  "PackageTags",
  "PackageVersion",
  "Platform",
  "PlatformTarget",
  "PublishAot",
  "PublishReadyToRun",
  "PublishSingleFile",
  "PublishTrimmed",
  "RepositoryUrl",
  "RootNamespace",
  "RuntimeIdentifier",
  "RuntimeIdentifiers",
  "SelfContained",
  "SignAssembly",
  "TargetFramework",
  "TargetFrameworks",
  "TreatWarningsAsErrors",
  "UseArtifactsOutput",
  "UserSecretsId",
  "UseWindowsForms",
  "UseWPF",
  "Version",
  "VersionPrefix",
  "VersionSuffix",
  "WarningLevel",
  "WarningsAsErrors",
  "WarningsNotAsErrors",
];

/** Properties MSBuild defines (read in `$(…)`, not set). */
const RESERVED_PROPERTIES = [
  "MSBuildProjectDirectory",
  "MSBuildProjectFile",
  "MSBuildProjectName",
  "MSBuildThisFileDirectory",
  "MSBuildThisFile",
  "MSBuildThisFileName",
  "MSBuildStartupDirectory",
  "MSBuildToolsPath",
  "MSBuildExtensionsPath",
  "MSBuildRuntimeType",
  "OutDir",
  "TargetDir",
  "TargetPath",
  "TargetFileName",
  "ProjectDir",
  "SolutionDir",
];

export const ITEMS = [
  "AdditionalFiles",
  "AssemblyAttribute",
  "Compile",
  "Content",
  "EmbeddedResource",
  "Folder",
  "FrameworkReference",
  "GlobalPackageReference",
  "InternalsVisibleTo",
  "None",
  "PackageReference",
  "PackageVersion",
  "ProjectReference",
  "Reference",
  "Using",
];

export const TARGETS = [
  "AfterBuild",
  "BeforeBuild",
  "Build",
  "Clean",
  "CopyFilesToOutputDirectory",
  "CoreBuild",
  "CoreCompile",
  "GenerateNuspec",
  "Pack",
  "PrepareForBuild",
  "Publish",
  "Rebuild",
  "ResolveReferences",
  "Restore",
  "Test",
  "VSTest",
];

export const TASKS = [
  "CallTarget",
  "Copy",
  "Delete",
  "Error",
  "Exec",
  "ItemGroup",
  "MakeDir",
  "Message",
  "MSBuild",
  "PropertyGroup",
  "ReadLinesFromFile",
  "RemoveDir",
  "Touch",
  "Warning",
  "WriteLinesToFile",
  "ZipDirectory",
];

const TOP_LEVEL = [
  "Choose",
  "Import",
  "ImportGroup",
  "ItemDefinitionGroup",
  "ItemGroup",
  "PropertyGroup",
  "Sdk",
  "Target",
  "UsingTask",
];

const CHILDREN: Record<string, readonly string[]> = {
  project: TOP_LEVEL,
  propertygroup: PROPERTIES,
  itemgroup: ITEMS,
  target: TASKS,
  choose: ["When", "Otherwise"],
  when: ["Choose", "ItemGroup", "PropertyGroup"],
  otherwise: ["Choose", "ItemGroup", "PropertyGroup"],
  importgroup: ["Import"],
};

/** Elements that take an `Include` (items) rather than text content. */
const ITEM_SET = new Set(ITEMS.map((i) => i.toLowerCase()));

const withoutComments = (text: string) => text.replace(/<!--[\s\S]*?(?:-->|$)/g, (c) => c.replace(/[^\n]/g, " "));

/** The element enclosing the end of `text` (lower-cased), or null at the top. */
export function enclosingElement(text: string): string | null {
  const stack: string[] = [];
  for (const m of withoutComments(text).matchAll(/<(\/?)([A-Za-z_][\w.:-]*)((?:[^>"']|"[^"]*"|'[^']*')*?)(\/?)>/g)) {
    const name = m[2]!.toLowerCase();
    if (m[1]) {
      const i = stack.lastIndexOf(name);
      if (i >= 0) stack.length = i;
    } else if (!m[4]) stack.push(name);
  }
  return stack.at(-1) ?? null;
}

/** Names the file defines: properties set in a `PropertyGroup`, item types, targets. */
export function definedNames(text: string): { properties: string[]; items: string[]; targets: string[] } {
  const clean = withoutComments(text);
  const children = (group: string) => {
    const out = new Set<string>();
    for (const g of clean.matchAll(new RegExp(`<${group}\\b[^>]*>([\\s\\S]*?)</${group}>`, "gi")))
      for (const m of g[1]!.matchAll(/<([A-Za-z_][\w.]*)\b/g)) out.add(m[1]!);
    return [...out];
  };
  const targets = [...clean.matchAll(/<Target\s[^>]*?\bName\s*=\s*["']([^"']+)["']/g)].map((m) => m[1]!.trim());
  return { properties: children("PropertyGroup"), items: children("ItemGroup"), targets };
}

const unique = (...lists: ReadonlyArray<readonly string[]>) => [...new Set(lists.flat())];

/** Inserts `<Name>|</Name>` (or `<Item Include="|" />`) for an element name typed after `<`. */
function elementApply(name: string, item: boolean) {
  return (view: EditorView, _c: Completion, from: number, to: number) => {
    const after = view.state.sliceDoc(to, to + 1);
    // Completing the name of a tag that is already written: just the name.
    if (/[\s>/]/.test(after)) {
      view.dispatch({ changes: { from, to, insert: name }, selection: EditorSelection.cursor(from + name.length) });
      return;
    }
    const insert = item ? `${name} Include="" />` : `${name}></${name}>`;
    const caret = item ? from + name.length + ' Include="'.length : from + name.length + 1;
    view.dispatch({ changes: { from, to, insert }, selection: EditorSelection.cursor(caret) });
  };
}

const options = (names: readonly string[], type: string, apply?: (n: string) => Completion["apply"]): Completion[] =>
  names.map((label) => ({ label, type, icon: null, ...(apply ? { apply: apply(label) } : {}) }));

/** Completions at `pos` of the MSBuild `text`. */
export function msbuildCompletions(text: string, pos: number): CompletionResult | null {
  const before = text.slice(0, pos);
  const lineStart = before.lastIndexOf("\n") + 1;
  const line = before.slice(lineStart);
  const defined = definedNames(text);
  const ref = /([$@])\(([\w.]*)$/.exec(line);
  if (ref) {
    const names =
      ref[1] === "$" ? unique(defined.properties, PROPERTIES, RESERVED_PROPERTIES) : unique(defined.items, ITEMS);
    return {
      from: pos - ref[2]!.length,
      options: options(names, ref[1] === "$" ? "property" : "type"),
      validFor: /^[\w.]*$/,
    };
  }
  const list =
    /\b(?:DependsOnTargets|AfterTargets|BeforeTargets|Targets|InitialTargets|DefaultTargets)\s*=\s*"([^"]*)$/i.exec(
      line,
    );
  if (list) {
    const word = /[\w.-]*$/.exec(list[1]!)![0];
    return {
      from: pos - word.length,
      options: options(unique(defined.targets, TARGETS), "function"),
      validFor: /^[\w.-]*$/,
    };
  }
  const tag = /<([A-Za-z_][\w.]*)?$/.exec(line);
  if (tag) {
    const parent = enclosingElement(before.slice(0, before.length - tag[0].length));
    const known = parent ? CHILDREN[parent] : null;
    if (!known) return null;
    const names = unique(
      parent === "propertygroup" ? defined.properties : parent === "itemgroup" ? defined.items : [],
      known,
    );
    const item = parent === "itemgroup";
    return {
      from: pos - (tag[1]?.length ?? 0),
      options: options(names, item ? "type" : parent === "target" ? "function" : "property", (n) =>
        elementApply(n, item && ITEM_SET.has(n.toLowerCase())),
      ),
      validFor: /^[\w.]*$/,
    };
  }
  return null;
}

const source: CompletionSource = (ctx: CompletionContext) => {
  const r = msbuildCompletions(ctx.state.doc.toString(), ctx.pos);
  if (!r) return null;
  // After `<` or `$(` the list opens by itself; elsewhere only on a typed word or explicitly.
  if (!ctx.explicit && r.from === ctx.pos && !/[<(]$/.test(ctx.state.sliceDoc(ctx.pos - 1, ctx.pos))) return null;
  return r;
};

/** Editor contribution: MSBuild completion in project, `.props` and `.targets` files. */
export function msbuildCompletion(ctx: EditorDocContext): Extension {
  if (isVirtualUri(ctx.path) || !MSBUILD_FILE.test(ctx.path)) return [];
  return EditorState.languageData.of(() => [{ openideaCompletion: source }]);
}
