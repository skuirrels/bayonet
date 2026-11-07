# Bayonet: specification for a Rider-class .NET IDE

Status: draft 1, 2026-10-31; progress per requirement and platform updated 2026-11-06 (sections 5 and 6)
Reference product: JetBrains Rider 2026.2.4 (build 262.11879.110), installed at `~/Applications/Rider.app`
Port source: OpenIDEA at `/Users/tyomidi/Source/openidea` (v0.1.29, commit `4796265`, Apache 2.0, Rust core + Tauri + React/CodeMirror 6)

## 0. How this spec was produced

The investigation followed the REA (`reverse-engineer-anything`) workflow: a question checklist, a findings ledger that separates observations from inferences and unknowns, and evidence tied to concrete paths.

**Clean-room boundary.** Rider is proprietary and its licence forbids decompiling or disassembling it. This spec therefore uses only:

- metadata a user can read without decompiling: `product-info.json`, `Info.plist`, directory layout, plugin folder names, assembly file names and the bundled third-party licence list;
- Rider's observable behaviour and public JetBrains documentation;
- open-source projects (OpenIDEA, Roslyn, NuGet, MSBuild, netcoredbg, ILSpy and others).

No Rider assembly or jar was decompiled, and nothing in this document is derived from Rider code. Keep it that way: contributors must not decompile Rider, copy its resources (icons, settings schemas, inspection texts, live templates) or reuse its trademarks. Matching keymaps and general UX conventions is fine; copying artwork and text is not.

## 1. Question checklist

| #   | Question                                                                                        | Status                  |
| --- | ----------------------------------------------------------------------------------------------- | ----------------------- |
| Q1  | What is Rider's process architecture?                                                           | Answered (O + I)        |
| Q2  | Which feature areas does Rider ship for .NET?                                                   | Answered (O)            |
| Q3  | Which open-source components does Rider itself rely on, and so are safe, proven choices for us? | Answered (O)            |
| Q4  | What does OpenIDEA already provide, and where does .NET plug in?                                | Answered (O), section 4 |
| Q5  | What must be built from scratch versus composed from open source?                               | Answered (I)            |
| Q6  | What is the phased scope to reach a usable Rider alternative?                                   | Answered (I), section 7 |

## 2. Findings ledger: Rider

O = observed in the install, I = inferred, U = unknown.

### 2.1 Process architecture (Q1)

| ID | Type | Finding | Evidence |
| --- | --- | --- | --- | --- | ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| R1 | O | The UI is an IntelliJ Platform (JVM) app running on a bundled JetBrains Runtime, Java 25 minimum. | `Contents/Resources/product-info.json` (`minRequiredJavaVersion: 25`, `jbr/`) |
| R2 | O | The language engine is a separate .NET process, the "ReSharper Host", shipped with its own .NET 10.0.9 runtime per platform. | `lib/ReSharperHost/` (898 entries), `lib/ReSharperHost/macos-arm64/dotnet/` |
| R3 | O | Frontend and backend talk over JetBrains' Rd protocol. | `JetBrains.RdFramework.dll`, `JetBrains.Platform.RdProtocol.dll`, `JetBrains.Rider.DotNetIpc.Protocol.dll` |
| R4 | I | The backend owns the .NET semantic model (PSI), solution model, MSBuild evaluation, NuGet, inspections, refactorings, unit testing and debugging. The JVM frontend owns editor, UI, VCS, database, web tooling and the plugin ecosystem. | Assembly families below, plugin folders |
| R5 | O | Separate worker processes exist for the debugger, process enumeration, MSBuild and test running. | `macos-arm64/JetBrains.Debugger.Worker`, `JetBrains.ProcessEnumerator.Worker`, `JetBrains.Platform.MsBuildHost.dll`, `TestRunner/` |
| R6 | O | Rider also hosts Roslyn in the backend, alongside its own C# engine. | `JetBrains.Rider.Roslyn.Host.dll`, `Microsoft.CodeAnalysis.*` |
| R7 | O | 140 frontend plugin folders, 481 entries in `lib/`. Install size 6.3 GB. | `Contents/plugins`, `du` |

### 2.2 Feature areas (Q2)

Derived from backend assembly families (names only) and frontend plugin folders.

| ID | Area | Rider evidence (names) |
| --- | --- | --- | --- | --- | --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F1 | Languages: C#, VB.NET, F#, XAML, Razor/Blazor (cshtml), ASP.NET WebForms, RESX, MSBuild, Protobuf, RegExp, XML, IL, C++ | `ReSharper.Psi.{CSharp,VB,Xaml,Web,Resx,BuildScripts,Protobuf,RegExp,IL,Asm,Xml}`, `rider-plugins-fsharp`, `rider-plugins-razor`, `rider-cpp` |
| F2 | Solution and project model: sln, slnx, slnf, csproj, fsproj, vbproj, sqlproj, shproj, props/targets | `Info.plist` document types, `Platform.ProjectModel*`, `Microsoft.VisualStudio.SolutionPersistence` |
| F3 | Build: MSBuild host, binary log, build events, solution builder | `Platform.MsBuildHost`, `Platform.MsBuildEventLogger`, `Platform.BuildEngine`, `Features.SolBuilderDuo` |
| F4 | NuGet: browse, install, restore, private feed auth, Package Manager Console | `Platform.ProjectModel.NuGet`, `NuGet.*`, `rider-plugins-privateFeedAuth`, `rider-pmc` |
| F5 | Code analysis ("daemon"): on-the-fly inspections, solution-wide analysis, spell checking | `ReSharper.Daemon.*`, `ReSharper.SolutionAnalysis`, `Features.ReSpeller` |
| F6 | Quick-fixes, context actions | `ReSharper.Intentions.*` |
| F7 | Refactorings: rename, move, extract, change signature, in-place | `ReSharper.Refactorings.*`, `InplaceRefactorings`, `NewRefactorings` |
| F8 | Navigation and search: go to symbol/type/file, find usages, structural search | `Features.Navigation`, `Features.StructuralSearch`, `Lucene.Net` |
| F9 | Completion, parameter info, live templates, postfix templates | `Features.Intellisense`, `ReSharper.LiveTemplates` |
| F10 | Decompiled and external sources: navigate into compiled assemblies, Source Link | `ReSharper.ExternalSources*`, `ICSharpCode.Decompiler` |
| F11 | Unit testing: xUnit, NUnit, MSTest, VSTest, coverage | `ReSharper.UnitTest*`, `TestRunner/Adapters`, `Microsoft.TestPlatform.*`, `dotCover` |
| F12 | Debugger: .NET (CoreCLR, Mono, Unity), expression evaluation, LINQ visualiser, parallel stacks, data grid | `Debugger.Host`, `Debugger.CorApi`, `Mono.Debugging.*`, `Microsoft.CodeAnalysis.*ExpressionEvaluator*`, `debuggerLinq`, `cidr-parallelStacks`, `rider-plugins-debuggerGrid` |
| F13 | Profiling: performance and memory, dynamic program analysis | `dotTrace.dotMemory`, `dpa`, `tools/profiler` |
| F14 | Hot reload / watch | `rider-plugins-dotnetwatch` |
| F15 | Designers: XAML previewer, WinForms designer | `Features.Xaml.Previewer.Host`, `Features.WinForms.Designer.*`, `rider-plugins-xaml-preview` |
| F16 | Framework and engine integrations: Entity Framework, .NET Aspire, Unity, Godot, Unreal, MAUI/Xamarin | `rider-efcore`, `aspire-plugin`, `rider-unity`, `rider-godot`, `UnrealLink` |
| F17 | Web front end: JS/TS, React, Angular, Vue, CSS, HTTP client | `javascript-plugin`, `react-plugin`, `angular-plugin`, `vuejs-plugin`, `restClient` |
| F18 | Platform: Git/GitHub/GitLab/Azure DevOps/Perforce, terminal, database tools, Docker/Kubernetes, remote development, settings sync | `vcs-*`, `terminal`, `DatabaseTools`, `clouds-*`, `remote-dev-server`, `settingsSync` |
| F19 | Keymaps: Visual Studio, VS 2022, VS Code, ReSharper, Visual Assist | `keymap-*` |
| F20 | AI: completion ranking, full-line completion, AI assistant services, MCP server, ACP | `completionMlRanking`, `fullLine`, `ReSharper.Feature.Services.AI`, `mcpserver`, `platform-acp-plugin` |
| F21 | i18n: resource localisation analysis | `ReSharper.I18n.Services.*` |

### 2.3 Open-source components Rider itself bundles (Q3)

Rider's own third-party licence list has 614 entries (`license/third-party-libraries.json`). The ones that matter for a .NET IDE, all observed as file names in `lib/ReSharperHost/`:

| Component                                                                                   | Licence    | Use for us                                                                         |
| ------------------------------------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------- |
| Roslyn (`Microsoft.CodeAnalysis.*`, incl. `LanguageServer.Protocol`, `ExpressionEvaluator`) | MIT        | C#/VB semantic model, diagnostics, code fixes, refactorings, expression evaluation |
| `Microsoft.VisualStudio.SolutionPersistence`                                                | MIT        | Read and write sln and slnx                                                        |
| NuGet client (`NuGet.*`)                                                                    | Apache 2.0 | Package search, restore, install, versioning                                       |
| `ICSharpCode.Decompiler` (ILSpy)                                                            | MIT        | Decompiled sources                                                                 |
| Mono.Cecil, `System.Reflection.Metadata`, DiaSymReader                                      | MIT        | Metadata and PDB reading                                                           |
| VSTest (`Microsoft.TestPlatform.*`)                                                         | MIT        | Test discovery and execution                                                       |
| `Microsoft.Diagnostics.NETCore.Client`                                                      | MIT        | Diagnostics IPC: attach, traces, dumps, counters                                   |
| Mono.Debugging.Soft                                                                         | MIT        | Mono and Unity soft debugger                                                       |
| Lucene.Net                                                                                  | Apache 2.0 | Full-text index (OpenIDEA may already cover this)                                  |

Inference: the parts of Rider we cannot reuse are its own C# engine (PSI, daemon, ReSharper inspections) and its CoreCLR debugger. Roslyn replaces the first and netcoredbg (MIT, DAP-native) the second.

## 3. Target architecture

OpenIDEA is LSP-first with a tree-sitter fallback and no PSI of its own (section 4). The port keeps that shape: Rider's ReSharper Host is replaced by Roslyn's language server plus a small .NET sidecar, both driven from the existing Rust core.

```
+----------------------- Bayonet (ported from OpenIDEA) -----------------------+
|  React 19 + CodeMirror 6 UI   src/features/*  (+ dotnet: Solution Explorer,  |
|                               NuGet, Unit Tests, .NET run/debug forms)       |
|  Rust core  crates/ide-*      ide-host, ide-lsp, ide-dap, ide-run, ide-sdk,  |
|                               ide-build, ide-workspace  (+ new ide-dotnet)   |
+-----------+------------------------------+------------------------+---------+
            | LSP (stdio)                  | JSON-RPC (ide-jsonrpc) | DAP
+-----------v-------------+  +-------------v-------------+  +-------v--------+
| Roslyn language server  |  | Bayonet.Host (.NET 10)    |  | netcoredbg     |
| Microsoft.CodeAnalysis. |  |  - sln/slnx/slnf edits    |  | (coreclr)      |
| LanguageServer (MIT)    |  |    (SolutionPersistence)  |  | Mono soft dbg  |
|  + Razor cohosting      |  |  - MSBuild evaluation     |  | later          |
|  + its own BuildHost    |  |  - NuGet client           |  +----------------+
+-------------------------+  |  - VSTest / MTP discovery |
  fsautocomplete (F#)        |  - ILSpy decompiler       |
  lemminx (XML/MSBuild)      +---------------------------+
```

Design rules:

- **Reuse OpenIDEA's protocol clients.** Roslyn LSP goes through `ide-lsp` like jdtls does today; the sidecar uses `ide-jsonrpc`; netcoredbg goes through `ide-dap`. No new transport, and nothing like Rider's Rd protocol.
- **Use Roslyn's language server unmodified**, downloaded on request by `ide-tools`, replacing `csharp-ls` in `languages/csharp.toml`. Its non-standard messages (`solution/open`, `project/open`, project initialisation status, source-generated documents, decompiled sources) get a server-specific module `crates/ide-lsp/src/roslyn.rs`, modelled on `jdtls.rs`. Fork it only if an extension point is missing.
- **The sidecar covers what LSP does not**: solution structure and edits, NuGet, test discovery and execution, and decompilation for assemblies Roslyn will not open. It is a separate process so it can crash or restart without affecting editing (NFR-4).
- **Rust owns UI-facing state.** Files, buffers, run configurations (`.bayonet/run.toml`) and the solution tree cache live in the Rust core. The .NET processes are services, not sources of truth.
- **Resolve, do not bundle, what the user has**: the dotnet SDK and MSBuild through `global.json`. Download on request, as OpenIDEA does today, anything the SDK does not provide: Roslyn LSP, netcoredbg, fsautocomplete. Never use vsdbg or C# Dev Kit (K3).
- **Use the latest .NET SDK (10.0.4xx on this machine) and latest stable NuGet packages** throughout, pinned and updated per release.

## 4. Porting from OpenIDEA

### 4.1 Findings ledger: OpenIDEA

| ID | Type | Finding | Evidence |
| ---- | --- | --- | --- | --- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| OI1 | O | Code insight comes from LSP first, with tree-sitter as the fallback. There is no PSI. The index covers only files and symbols. | `crates/ide-lsp`, `crates/ide-syntax/src/lib.rs`, `crates/ide-index` |
| OI2 | O | Each language is a single TOML file declaring grammar, LSP servers and debugger. C# already uses csharp-ls and netcoredbg; F# uses fsautocomplete. | `languages/csharp.toml`, `languages/fsharp.toml` |
| OI3 | O | XML already claims csproj, props, targets, nuspec, xaml, axaml and resx through lemminx. VB, Razor, a dedicated MSBuild language and sln are missing. | `languages/xml.toml` |
| OI4 | O | netcoredbg installs and resolves, but it is hidden from the run form because it has no verified launch template. | `src/features/run/ConfigForm.tsx:47`, `crates/ide-dap/src/adapter.rs` |
| OI5 | O | There is no .NET entry in `SdkKind` or `BuildSystem`, no .NET run detection and no .NET fixture. | `crates/ide-sdk/src/lib.rs:33`, `crates/ide-build/src/compile/mod.rs:34`, `crates/ide-run/src/detect.rs`, `fixtures/` |
| OI6 | O | The Java integration (jdtls extensions, java-debug, hot swap) is the most complete template for a JVM-grade language. The Go workflow commit `ac360b6` (57 files, about 2.6k lines) lists everything a new language workflow touches. | `crates/ide-lsp/src/jdtls.rs`, `src/features/debugger/java.ts`, `git show ac360b6` |
| OI7 | O | The database client already supports SQL Server through tiberius. | `crates/ide-db` |
| OI8 | O | The WASM plugin ABI is narrow (commands, notifications, file read, text insert, register language or theme). It cannot host .NET tooling. | `wit/plugin.wit` |
| OI9 | O | About 870 Tauri commands. Bindings are generated with specta into `src/bindings.ts`, and CI fails if they are out of date. | `src-tauri/src/commands/`, `pnpm gen:bindings` |
| OI10 | O | `NOTICE` credits IntelliJ Platform code and icons (Apache 2.0). OpenIDEA drew its own language icons rather than using JetBrains' proprietary ones. `deny.toml` allows only permissive licences and MPL-2.0. | `NOTICE`, `THIRD_PARTY_LICENSES.md`, `deny.toml` |

U1 is resolved: OpenIDEA has no PSI, so semantic depth depends entirely on the language server. Roslyn LSP is therefore the decisive component, and K1 stands as the main parity risk.

### 4.2 Port mechanics

1. **Bring the history across.** Clone `/Users/tyomidi/Source/openidea` into `dotnetidea` with full history, then rename the remote to `upstream`, so OpenIDEA fixes can still be merged. Record the base commit (`4796265`) in `docs/porting.md`.
2. **Rebrand in one commit.** Change product name, bundle id, icons, the `openidea-server` binary name, the `.openidea/` project directory (to `.bayonet/`, migrating existing ones), locale strings and the website links. Keep identifiers in Rust module paths for now; renaming those makes upstream merges harder.
3. **Keep attribution.** Preserve `LICENSE` and `NOTICE`, add a line crediting OpenIDEA, and keep `THIRD_PARTY_LICENSES.md` current. Keep `deny.toml` as it is.
4. **Change the default focus.** Make the Visual Studio keymap and the .NET language set the defaults. Other languages stay available; removing them buys little and makes merges harder.
5. **CI stays as is** (fmt, clippy, nextest on three OSes, cargo-deny, Playwright). Add a job that installs the .NET 10 SDK and runs the .NET fixtures.

### 4.3 Port map

| OpenIDEA area                                                                                                                                                 | Action            | .NET work                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ide-core`, `ide-vfs`, `ide-text`, `ide-jsonrpc`, `ide-search`, `ide-index`, `ide-syntax`, `ide-analysis`                                                     | Port as is        | None                                                                                                                                                                                                              |
| `ide-terminal`, `ide-vcs`, `ide-github`, `ide-http`, `ide-docker`, `ide-ssh`, `ide-remote`, `ide-spell`, `ide-fonts`, `ide-clipboard`, `ide-web`, `ide-agent` | Port as is        | `.http` compatibility with Visual Studio's format (PLT-3)                                                                                                                                                         |
| `ide-db`                                                                                                                                                      | Port as is        | SQL Server already works; add EF Core connection string discovery from `appsettings*.json`                                                                                                                        |
| `ide-language`, `languages/*.toml`                                                                                                                            | Adapt             | Swap csharp-ls for Roslyn LSP. Add `vb.toml`, `razor.toml`, `msbuild.toml` (lemminx with MSBuild schema), `sln.toml`, `slnx` under XML. Add a C# grammar upgrade and F# tree-sitter grammar if one is maintained. |
| `ide-lsp`                                                                                                                                                     | Adapt             | New `roslyn.rs`: solution open, project load progress, source-generated and decompiled virtual documents (reusing `virtual_doc.rs`), Razor cohosting                                                              |
| `ide-tools`                                                                                                                                                   | Adapt             | Installer entry for Roslyn LSP (NuGet package per RID), keep netcoredbg and fsautocomplete                                                                                                                        |
| `ide-sdk`                                                                                                                                                     | Adapt             | `SdkKind::Dotnet`: discover SDKs and runtimes (`dotnet --list-sdks`, `DOTNET_ROOT`, `global.json` roll-forward)                                                                                                   |
| `ide-workspace`                                                                                                                                               | Adapt             | `roots/dotnet.rs`: content roots from projects, `bin/` and `obj/` excluded                                                                                                                                        |
| `ide-build`                                                                                                                                                   | Adapt             | `BuildSystem::Dotnet`: `dotnet build` with a binary-log logger for structured errors; build progress per project                                                                                                  |
| `ide-run`                                                                                                                                                     | Adapt             | `detect_dotnet`: executable projects and `launchSettings.json` profiles; `dotnet watch` mode; Aspire AppHost                                                                                                      |
| `ide-dap` + `src/features/debugger`                                                                                                                           | Adapt             | `coreclr` launch and attach template, add to `ADAPTERS`, tests in `launch.test.ts`                                                                                                                                |
| `src/features/run/testing`, `ide-run/src/junit.rs`                                                                                                            | Adapt             | TRX parser, and live results from the sidecar's test service                                                                                                                                                      |
| `ide-coverage`                                                                                                                                                | Adapt             | Coverlet (Cobertura or lcov)                                                                                                                                                                                      |
| `ide-profiler`                                                                                                                                                | Adapt (P2)        | `dotnet-trace` and `dotnet-counters` plans and the nettrace format                                                                                                                                                |
| `ide-host/src/ops`                                                                                                                                            | Adapt             | New `ops/dotnet.rs`: solution tree, NuGet, tests, templates, all proxied to the sidecar                                                                                                                           |
| `src-tauri`                                                                                                                                                   | Adapt             | Commands for the above, regenerated bindings, rebranding                                                                                                                                                          |
| `src/features/project-tree`                                                                                                                                   | Adapt             | Solution Explorer mode: solution folders, projects, Dependencies node (packages, projects, frameworks, analysers, source generators)                                                                              |
| `src/features/build-tools`                                                                                                                                    | Adapt             | NuGet tool window, using the Maven/Gradle window as its model                                                                                                                                                     |
| `src/platform/keymap.ts`                                                                                                                                      | Adapt             | Visual Studio keymap, set as default                                                                                                                                                                              |
| `src/editor/refactor/catalog.ts`                                                                                                                              | Adapt             | Map Roslyn code-action kinds and titles onto the refactoring menu                                                                                                                                                 |
| **New** `crates/ide-dotnet`                                                                                                                                   | Build             | sln/slnx/slnf reader (fast, Rust, for instant tree), sidecar process manager, caches                                                                                                                              |
| **New** `dotnet/Bayonet.Host`                                                                                                                                 | Build             | .NET 10 sidecar: SolutionPersistence, MSBuild evaluation (`Microsoft.Build` + MSBuildLocator), NuGet client, VSTest and Microsoft.Testing.Platform, ICSharpCode.Decompiler                                        |
| **New** `fixtures/dotnet/`                                                                                                                                    | Build             | Multi-project slnx with web API, class library, xUnit v3, NUnit and MSTest projects, CPM, multi-targeting, a source generator, an F# project and an Aspire AppHost                                                |
| `ide-build` Maven/Gradle, `ide-notebook`, Java-specific `ops/java.rs`                                                                                         | Keep, not default | No work; keep so upstream merges stay clean                                                                                                                                                                       |
| `ide-plugin`, `wit/plugin.wit`                                                                                                                                | Port as is        | None for .NET (OI8); later ABI growth is an upstream concern                                                                                                                                                      |

## 5. Functional specification

Each requirement has an ID, a priority (P0 = MVP, P1 = parity for daily use, P2 = later) and an acceptance test.

Bayonet must run on macOS, Windows and Linux (NFR-5), so each requirement is tracked per platform. Status is as of 2026-11-06, taken from `docs/porting.md` and the code:

- **Status**: Done (the requirement is built), Partial (part of it is missing; the notes under the table say what), Not done, or Out of scope.
- **macOS, Windows, Linux**: Yes (built and tested on that platform), Untested (the code has that platform's paths but has not been run there), No (does not work there).
- Everything marked Yes was checked on macOS arm64 only. The CI matrix in `.github/workflows/ci.yml` covers `macos-latest`, `ubuntu-22.04` and `windows-latest`. Since 2026-11-06 it runs on every push to `master` and `dotnet-port` and on pull requests to `master`. The first such run is 42310553128.

### 5.1 Solution and project model

| ID    | Pri | Status | macOS | Windows  | Linux    | Requirement                                                                                                                                                                | Acceptance                                                                                                      |
| ----- | --- | ------ | ----- | -------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| SLN-1 | P0  | Done   | Yes   | Untested | Untested | Open a folder, `.sln`, `.slnx` or `.slnf`; show Solution Explorer with solution folders, projects, dependencies (packages, project refs, frameworks, analysers) and files. | Opening `fixtures/dotnet/MultiProject.slnx` shows every project and its dependencies within 3 s on a warm cache |
| SLN-2 | P0  | Done   | Yes   | Untested | Untested | Evaluate projects with MSBuild through the user's installed SDK (respect `global.json`), in an out-of-process build host.                                                  | Changing `global.json` to another installed SDK re-evaluates with that SDK                                      |
| SLN-3 | P0  | Done   | Yes   | Untested | Untested | Multi-targeting: a target-framework switcher per editor, analysis per TFM.                                                                                                 | `#if NET10_0` regions grey out correctly when switching TFM                                                     |
| SLN-4 | P1  | Done   | Yes   | Untested | Untested | Add, remove, rename projects and files, edit project references; write back via SolutionPersistence and MSBuild project APIs, preserving formatting.                       | Round-trip of a hand-formatted csproj changes only the edited lines                                             |
| SLN-5 | P1  | Done   | Yes   | Untested | Untested | `Directory.Build.props/targets`, `Directory.Packages.props` (central package management) understood and navigable.                                                         | Go to definition on a CPM package version lands in `Directory.Packages.props`                                   |
| SLN-6 | P1  | Done   | Yes   | Untested | Untested | Convert sln to slnx.                                                                                                                                                       | Produces a slnx accepted by `dotnet build`                                                                      |
| SLN-7 | P1  | Done   | Yes   | Untested | Untested | Project templates via `dotnet new` (list, parameters, create).                                                                                                             | New Project dialog lists installed templates, including custom ones                                             |

- SLN-1: Dependencies lists analysers (with their generated files), frameworks, packages (with the packages they bring in) and project references, per target framework when a project has several. They are read from `obj/project.assets.json`, or from the project file before the first restore. The 3-second acceptance time has not been measured.
- SLN-5: properties set in SDK targets or imported files are not followed yet.

### 5.2 Build and run

| ID    | Pri | Status | macOS | Windows  | Linux    | Requirement                                                                                                                                | Acceptance                                                         |
| ----- | --- | ------ | ----- | -------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| BLD-1 | P0  | Done   | Yes   | Untested | Untested | Build, rebuild, clean solution or project; streaming output; errors in a Problems view linked to source.                                   | Introduced compile error appears in Problems and editor            |
| BLD-2 | P0  | Done   | Yes   | Untested | Untested | Run configurations auto-generated from `launchSettings.json` and executable projects; environment variables, arguments, working directory. | Each launch profile appears as a run configuration                 |
| BLD-3 | P1  | Done   | Yes   | Untested | Untested | Build progress per project, cancel build, binary log capture and viewer.                                                                   | Cancel stops MSBuild within 2 s                                    |
| BLD-4 | P1  | Done   | Yes   | Untested | Untested | `dotnet watch` / Hot Reload run mode.                                                                                                      | Editing a method body in a running web app applies without restart |
| BLD-5 | P1  | Done   | Yes   | Untested | Untested | .NET Aspire AppHost run: dashboard link, resource list, per-resource logs.                                                                 | Running an AppHost lists its resources                             |
| BLD-6 | P2  | Done   | Yes   | Untested | Untested | Publish profiles (folder, container).                                                                                                      | `dotnet publish` with a selected profile                           |

- BLD-3: the 2-second cancel test (`dotnet_build_cancels_within_two_seconds`) runs on Unix only. Windows cancels through `taskkill /T /F`, which has not been timed.

### 5.3 Code insight (C# first, then VB, F#)

| ID    | Pri | Status  | macOS | Windows  | Linux    | Requirement                                                                                                                     | Acceptance                                                                                 |
| ----- | --- | ------- | ----- | -------- | -------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| CI-1  | P0  | Done    | Yes   | Untested | Untested | Semantic highlighting, diagnostics (compiler + analysers + `.editorconfig` severities) live as you type.                        | Diagnostic for an unused variable appears under 300 ms after a keystroke on a 2k-line file |
| CI-2  | P0  | Done    | Yes   | Untested | Untested | Completion: symbols, members, keywords, unimported types with auto-using, override/partial completion, snippets.                | Typing `JsonSer` offers `JsonSerializer` and adds `using System.Text.Json;`                |
| CI-3  | P0  | Done    | Yes   | Untested | Untested | Go to definition, implementation, base, type definition; find usages grouped by kind; highlight usages.                         | Find usages on an interface method lists implementations and calls separately              |
| CI-4  | P0  | Done    | Yes   | Untested | Untested | Quick-fixes and context actions from Roslyn code fixes and refactoring providers (Alt+Enter).                                   | Alt+Enter on a missing using adds it                                                       |
| CI-5  | P0  | Partial | Yes   | Untested | Untested | Rename (symbols, files, namespaces), with preview.                                                                              | Renaming a class also renames its file when names match                                    |
| CI-6  | P1  | Done    | Yes   | Untested | Untested | Refactorings: extract method/variable/interface, inline, change signature, move type to file, introduce parameter, safe delete. | Each refactoring has a fixture test                                                        |
| CI-7  | P1  | Done    | Yes   | Untested | Untested | Solution-wide analysis: error count across solution, background, incremental.                                                   | Errors in unopened files appear in the Problems view                                       |
| CI-8  | P1  | Done    | Yes   | Untested | Untested | Code cleanup and reformat on save using `.editorconfig` and Roslyn formatter.                                                   | Reformat matches `dotnet format` output                                                    |
| CI-9  | P1  | Done    | Yes   | Untested | Untested | Inlay hints (parameter names, types), CodeLens-style usage and test markers, gutter icons for overrides and implementations.    | Hints toggle in settings                                                                   |
| CI-10 | P1  | Partial | Yes   | Untested | Untested | Navigate into referenced assemblies: Source Link first, then decompiled source via ICSharpCode.Decompiler, with IL view.        | Go to definition on `List<T>.Add` opens source or decompiled code                          |
| CI-11 | P1  | Done    | Yes   | Untested | Untested | Go to everything (Shift Shift) across types, symbols, files, actions; go to file member.                                        | Results under 100 ms on a 5k-file solution                                                 |
| CI-12 | P2  | Done    | Yes   | Untested | Untested | Structural search and replace for C#.                                                                                           | Pattern `$x$ == null` finds all null comparisons                                           |
| CI-13 | P2  | Done    | Yes   | Untested | Untested | Type hierarchy, call hierarchy, file structure, project dependency diagram.                                                     | Call hierarchy shows incoming calls                                                        |
| CI-14 | P1  | Done    | Yes   | Untested | Untested | F# via FsAutoComplete (MIT, LSP).                                                                                               | Diagnostics and completion in an fsproj                                                    |
| CI-15 | P2  | Done    | Yes   | Untested | Untested | Razor/Blazor via the Razor language server (MIT).                                                                               | Completion of components in `.razor`                                                       |
| CI-16 | P2  | Partial | Yes   | Untested | Untested | XAML (WPF, Avalonia, MAUI) completion and navigation; Avalonia previewer.                                                       | Binding path completion                                                                    |
| CI-17 | P1  | Done    | Yes   | Untested | Untested | Source generators: show generated files, navigate into them.                                                                    | Generated files appear under Dependencies > Analysers                                      |
| CI-18 | P1  | Done    | Yes   | Untested | Untested | MSBuild files: completion of properties, items, targets; go to imported target.                                                 | Go to definition on `<Target Name="Build">` reference                                      |

- CI-5: Rename (with preview) works for symbols and namespaces, but nothing renames a class's file to match. Roslyn's rename edit has no file operation, and the IDE does not add one.
- CI-8: whether Roslyn offers `source.organizeImports` and `source.fixAll` for cleanup on save has not been checked.
- CI-10: Source Link and decompiled sources work. There is no IL view.
- CI-16: binding paths, navigation and the Avalonia previewer work. Element and attribute completion goes no further than LemMinX's XML completion. WPF and MAUI have no previewer.

### 5.4 Unit testing

| ID    | Pri | Status  | macOS | Windows  | Linux    | Requirement                                                                                                                              | Acceptance                                                  |
| ----- | --- | ------- | ----- | -------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| TST-1 | P0  | Partial | Yes   | Untested | Untested | Discover tests (xUnit v2/v3, NUnit, MSTest) via VSTest and Microsoft.Testing.Platform; Unit Tests tool window with tree, filter, status. | Fixture solution with all three frameworks shows every test |
| TST-2 | P0  | Done    | Yes   | Untested | Untested | Run and debug single test, class, project, session; gutter run icons; output and stack traces linked to source.                          | Clicking a gutter icon runs only that test                  |
| TST-3 | P1  | Done    | Yes   | Untested | Untested | Continuous testing (rerun affected tests on build).                                                                                      | Editing a method reruns its tests                           |
| TST-4 | P1  | Done    | Yes   | Untested | Untested | Code coverage via coverlet or Microsoft.CodeCoverage, with editor highlighting.                                                          | Covered lines highlighted green                             |
| TST-5 | P2  | Done    | Yes   | Untested | Untested | Data-driven test cases shown as children (Theory, TestCase).                                                                             | Each InlineData row is a node                               |

- TST-1: only Microsoft.Testing.Platform test projects are supported. VSTest-only projects show a notice and get no tests, and so no coverage (TST-4).

### 5.5 Debugger

| ID    | Pri | Status       | macOS | Windows  | Linux    | Requirement                                                                                                                                                                        | Acceptance                                                |
| ----- | --- | ------------ | ----- | -------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| DBG-1 | P0  | Done         | Yes   | Untested | Untested | Launch and attach debugging of CoreCLR apps through netcoredbg over DAP: breakpoints (line, conditional, hit count, logpoints), step, pause, threads, call stack, locals, watches. | Debugging a console app stops at a conditional breakpoint |
| DBG-2 | P0  | Done         | Yes   | Untested | Untested | Exception breakpoints (thrown/unhandled, by type).                                                                                                                                 | Break on `InvalidOperationException` when thrown          |
| DBG-3 | P1  | Partial      | Yes   | Untested | Untested | Evaluate expressions and immediate window, including method calls and LINQ.                                                                                                        | `list.Where(x => x > 2).Count()` evaluates                |
| DBG-4 | P1  | Done         | Yes   | Untested | Untested | Attach to process picker (local .NET processes via diagnostics IPC).                                                                                                               | Lists running dotnet processes with names                 |
| DBG-5 | P1  | Done         | Yes   | Untested | Untested | Debug unit tests and Aspire resources.                                                                                                                                             | Breakpoint in a test hits                                 |
| DBG-6 | P2  | Partial      | Yes   | Untested | Untested | Parallel stacks, data tips, collection visualisers, set next statement.                                                                                                            | Visual parallel stacks for a multi-threaded app           |
| DBG-7 | P2  | Out of scope | No    | No       | No       | Unity and Mono via the soft debugger.                                                                                                                                              | Attach to a Unity editor                                  |
| DBG-8 | P2  | Partial      | Yes   | Untested | Untested | Remote and container debugging (netcoredbg over SSH/Docker exec).                                                                                                                  | Debug an app in a container                               |

Known gap: netcoredbg does not support Hot Reload and its expression evaluator is weaker than Rider's. Track [Samsung/netcoredbg](https://github.com/Samsung/netcoredbg); consider contributing a Roslyn-based evaluator.

- Platforms: the IDE installs netcoredbg from Samsung's releases, which have builds for macOS arm64, Windows x64 and Linux x64 and arm64 only. Debugging (DBG-1 to DBG-6, DBG-8) therefore does not work on Intel Macs or Windows on Arm until another build is found or made.
- DBG-3: netcoredbg calls instance methods but not static ones, so LINQ extension methods such as `Where` do not evaluate. The acceptance expression has not been tried.
- DBG-6: Parallel Stacks and collection visualisers work (a `Dictionary` shows nothing yet). Set Next Statement is built but netcoredbg 3.2 does not support it, so it is not offered for .NET. Data tips are not done.
- DBG-7: needs a Mono soft-debugger adapter; out of scope for now.
- DBG-8: container debugging works. SSH works through a launch configuration's `pipeTransport` only, with no UI, and netcoredbg must already be on the remote machine.

### 5.6 NuGet

| ID    | Pri | Status  | macOS | Windows  | Linux    | Requirement                                                                                                         | Acceptance                                                        |
| ----- | --- | ------- | ----- | -------- | -------- | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| NUG-1 | P0  | Partial | Yes   | Untested | Untested | Restore on open and on change; show restore errors.                                                                 | Missing package triggers restore                                  |
| NUG-2 | P1  | Done    | Yes   | Untested | Untested | NuGet tool window: search feeds, install, update, remove, per project or solution; respects `nuget.config` and CPM. | Updating a CPM package edits `Directory.Packages.props`           |
| NUG-3 | P1  | Done    | Yes   | Untested | Untested | Authenticated feeds via the user's credential providers (Azure Artifacts).                                          | Private feed lists packages                                       |
| NUG-4 | P2  | Done    | Yes   | Untested | Untested | Vulnerable and deprecated package warnings.                                                                         | `dotnet list package --vulnerable` results surface as diagnostics |

- NUG-1: packages are restored as part of every build, test run and package change, but not when a solution is opened or a project file changes. Roslyn's `workspace/_roslyn_projectNeedsRestore` request is not handled, and restore errors do not show on their own.

### 5.7 Profiling and diagnostics (P2)

| ID    | Pri | Status  | macOS | Windows  | Linux    | Requirement                                                                                | Acceptance                          |
| ----- | --- | ------- | ----- | -------- | -------- | ------------------------------------------------------------------------------------------ | ----------------------------------- |
| PRF-1 | P2  | Done    | Yes   | Untested | Untested | Live counters (CPU, GC, allocations) via EventPipe for a running app.                      | Counters chart updates every second |
| PRF-2 | P2  | Done    | Yes   | Untested | Untested | CPU sampling traces (`dotnet-trace`) with flame graph and call tree, navigating to source. | Hot method opens in editor          |
| PRF-3 | P2  | Partial | Yes   | Untested | Untested | Memory dumps (`dotnet-gcdump`) with type and retention view.                               | Largest types listed                |

- PRF-3: memory snapshots list types by size and compare two snapshots. There is no retention (what keeps an object alive) view. `docs/perf-tuning-plan.md` proposes the next profiling work.

### 5.8 Platform features (mostly inherited)

Git and hosting integrations, terminal, database client (including SQL Server), HTTP client, Docker, SSH and remote development, settings and keymaps are ported from OpenIDEA (section 4). .NET-specific additions:

| ID    | Pri | Status | macOS | Windows  | Linux    | Requirement                                                                               |
| ----- | --- | ------ | ----- | -------- | -------- | ----------------------------------------------------------------------------------------- |
| PLT-1 | P0  | Done   | Yes   | Untested | Untested | Visual Studio keymap as an alternative to the IntelliJ keymap.                            |
| PLT-2 | P1  | Done   | Yes   | Untested | Untested | EF Core: migrations add/update/remove, DbContext scaffolding via `dotnet ef`.             |
| PLT-3 | P1  | Done   | Yes   | Untested | Untested | `.http` files compatible with Visual Studio's format (variables, `http-client.env.json`). |
| PLT-4 | P2  | Done   | Yes   | Untested | Untested | User secrets editor (`dotnet user-secrets`).                                              |
| PLT-5 | P2  | Done   | Yes   | Untested | Untested | Docker: run and debug with the SDK container support (`PublishContainer`).                |

## 6. Non-functional requirements

| ID    | Status   | macOS   | Windows  | Linux    | Requirement                                                                                                                                                                                        |
| ----- | -------- | ------- | -------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| NFR-1 | Done     | Yes     | Untested | Untested | Open a 100-project solution to "ready" (all projects evaluated, diagnostics for open files) in under 15 s warm, measured with OpenIDEA's `perf/compare` harness against Rider on the same machine. |
| NFR-2 | Done     | Yes     | Untested | Untested | Memory for that solution under 1.5 GB total across all processes (Rider baseline to be measured).                                                                                                  |
| NFR-3 | Partial  | Yes     | Untested | Untested | Typing latency under 16 ms at p99; completion popup under 150 ms at p95.                                                                                                                           |
| NFR-4 | Done     | Yes     | Untested | Untested | The .NET host can crash or be restarted without losing editor state; the UI shows a degraded mode banner.                                                                                          |
| NFR-5 | Not done | Partial | Untested | Untested | Bayonet runs on macOS (arm64, x64), Windows (x64, arm64) and Linux (x64, arm64), and every requirement in section 5 works on each, except where a note says why it cannot.                         |
| NFR-6 | Done     | Yes     | Untested | Untested | No telemetry by default.                                                                                                                                                                           |
| NFR-7 | Done     | Yes     | Untested | Untested | All bundled components carry licences compatible with Apache 2.0 redistribution; `THIRD_PARTY_LICENSES.md` updated per release.                                                                    |

- NFR-1, NFR-2: measured on an M-series Mac only (Bayonet 10.1 s to ready and 791 MB, against Rider's 3712 MB).
- NFR-3: the perf harness reports p95, not p99. Typing p95 is 16.7 ms, at the 60 Hz frame, so the 16 ms p99 target is not shown to pass. The completion popup's p95 is 59 to 67 ms, well under 150 ms.
- NFR-5, what is missing for each platform:
  - **All:** the Windows and Linux jobs in the CI matrix have not passed yet. The first run (2026-11-06) was still in progress when this was written.
  - **macOS:** only arm64 has been run. The release kit builds an Intel `.dmg`, but no one has launched it. netcoredbg has no Intel build, so there is no debugging there (section 5.5).
  - **Windows:** the release kit builds x64 installers. Nothing has been launched or driven on Windows. There is no arm64 installer, and netcoredbg has no arm64 build.
  - **Linux:** the release kit builds x64 AppImage, `.deb` and `.rpm` packages in WSL. Nothing has been launched or driven on Linux. There are no arm64 packages. Remote development still downloads OpenIDEA's `openidea-server`, because no Bayonet server is published (it needs a musl build).

## 7. Delivery phases

Each phase ends with its acceptance tests passing in CI on the `fixtures/dotnet/` solution, on macOS, Windows and Linux. A requirement counts as done on a platform only once it has been run there (section 5's platform columns).

| Phase                   | Goal                                                                 | Requirements                                    | Main work (section 4.3)                                                                                                  |
| ----------------------- | -------------------------------------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| 0. Port                 | Bayonet builds, runs and passes OpenIDEA's own CI under the new name | none new                                        | Port mechanics 4.2, `fixtures/dotnet/`, .NET CI job                                                                      |
| 1. Edit                 | A C# solution opens with full Roslyn insight                         | SLN-1, SLN-2, CI-1 to CI-5, CI-11, NUG-1, PLT-1 | `SdkKind::Dotnet`, Roslyn LSP install and `roslyn.rs`, `ide-dotnet` slnx reader, Solution Explorer, Visual Studio keymap |
| 2. Build, run, debug    | Daily inner loop without leaving the IDE                             | BLD-1, BLD-2, DBG-1, DBG-2, SLN-3               | `BuildSystem::Dotnet`, `detect_dotnet`, coreclr launch template, TFM switcher                                            |
| 3. Test and packages    | Tests and NuGet at Rider's level of convenience                      | TST-1, TST-2, NUG-2, NUG-3, SLN-4, SLN-5, SLN-7 | `Bayonet.Host` sidecar (NuGet, VSTest/MTP, SolutionPersistence), NuGet window, Unit Tests window                         |
| 4. Parity for daily use | Remaining P1                                                         | all P1 IDs                                      | Refactoring catalogue, decompiled sources, solution-wide analysis, Hot Reload, Aspire, coverage, EF Core, F#             |
| 5. Beyond               | P2                                                                   | all P2 IDs                                      | Profiler, Razor, XAML, structural search, Unity/Mono, remote and container debugging                                     |

Phase 1 is the go/no-go point. If Roslyn LSP inside OpenIDEA's LSP layer cannot reach CI-1 to CI-5 and NFR-3, revisit the architecture before building phases 2 to 5.

## 8. Risks and open questions

| ID | Risk or question | Mitigation |
| --- | --- | --- | --- | --- | --------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| K1 | Roslyn LSP covers far fewer refactorings and inspections than ReSharper. Parity on code analysis is the largest gap. | Accept Roslyn plus community analysers (Roslynator, MIT) as the baseline; add IDE-specific refactorings in the host over time. |
| K2 | netcoredbg lags Rider's debugger (evaluation, Hot Reload, mixed mode). | Ship netcoredbg for P0; evaluate building a debugger on `ClrMD` and ICorDebug later. |
| K3 | Microsoft's vsdbg and C# Dev Kit are licensed only for Microsoft products and must not be bundled or launched. | Use only MIT/Apache components listed in 2.3 and section 4. |
| K4 | Roslyn's language server is not published as a stable library API. | Pin versions, keep a thin fork, upstream extension points. |
| K5 | Trademarks: "Rider", "ReSharper", "IntelliJ" must not appear in product naming or UI. | Product name Bayonet (working title); review before release. |
| K6 | Upstream drift: OpenIDEA is moving fast (424 commits, several a day). A long-lived port diverges. | Merge `upstream` at least weekly; keep .NET work in new files and crates where possible; avoid renaming shared modules. |
| K7 | Two semantic models disagree: the sidecar's MSBuild evaluation and Roslyn LSP's BuildHost may see different project states. | Treat Roslyn LSP as the source of truth for compilation; the sidecar only for structure and edits; reload both on project file change. |
| U2 | Unknown: whether Roslyn LSP's Razor cohosting works outside VS Code. | Spike in phase 4 before committing to CI-15. |
