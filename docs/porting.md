# Porting notes

Bayonet is a port of OpenIDEA. This file records how the two relate, so upstream
changes can keep flowing in.

## Base

- Upstream: <https://github.com/l0ng-ai/openidea>, remote `upstream`.
- Imported from the local checkout `/Users/tyomidi/Source/openidea` at `4796265`
  (v0.1.29), with full history.
- Merge `upstream/main` regularly (spec risk K6).

## Upstream merges

- **2026-11-04, `48e23c01` (v0.1.35 and 309 commits).** Conflicts were mostly two
  sides adding to the same list (CMake beside .NET, Ruby beside the .NET SDK kind,
  Docker beside `ops::dotnet`, plugin `pinned` beside the pull-diagnostics flags).
  Decisions worth knowing for the next merge:
  - **Upstream's .NET run configurations are removed** (`245fe3cd`, #686, part of
    upstream issue #615). They registered the same `dotnet` type id as ours, and also
    `dotnet-test`, with their own project detection (`crates/ide-run/src/detect/dotnet.rs`,
    `crates/ide-run/src/dotnet.rs`), a `run_dotnet_resolve` command, a "Build .NET
    project" before-launch task and `src/features/run/dotnetTypes.ts`. Ours stays the
    only one: detection in `crates/ide-run/src/detect.rs`, the type in
    `src/features/dotnet/runType.ts`, debugging through `crates/ide-dap/src/dotnet.rs`.
    When upstream changes those files again, the merge reports modify/delete
    conflicts: keep them deleted, and port anything useful into ours instead. Their
    `file-csharp` run icon, the `netcoredbg` missing-adapter notice and the
    netcoredbg entry in the debugger list are kept.
  - **Reformat indent.** Upstream now sends the Code Style indent only when the
    language has a Tabs and Indents override (`fcfe2647`). Bayonet keeps sending it
    always (`DocModel.formatIndent`), as recorded under Roslyn features, but uses
    upstream's `formatIndentFacet` and the indent re-detection after Reformat Code
    (`c599265c`).
  - **Virtual documents.** Roslyn source-generated files are a branch beside upstream's
    Kotlin `decompile` in `virtual_document`, and go through upstream's
    `store_virtual`, which skips the cache for them.
  - App icons, product name, identifier and version in `tauri.conf.json`, and the
    README stay ours.
- **2026-11-05, `9669e583` (v0.1.36 and 86 commits).**
  - **Pull diagnostics.** Upstream now pulls `textDocument/diagnostic` from every server
    that advertises `diagnosticProvider` (`20c439ba`), keeping pulled and pushed
    diagnostics apart, and declares pull support to all servers. `crates/ide-lsp/src/pull.rs`
    is upstream's document pull plus our `workspace/diagnostic` loop. `pull-diagnostics`
    now only adds servers that did not advertise the capability. The workspace pull's
    findings are stored as the server's pushed diagnostics. A file's own pull clears them
    while it is open, and on close its pulled findings are kept as pushed ones, since
    Roslyn reports a closed file again only when it changes.
  - **Unity.** Upstream's Unity support (`7e9acdc5`) is kept. The Roslyn server gets the
    same `ProjectSettings/ProjectVersion.txt` root marker as csharp-ls, and the Project
    view lists Solution and Unity panes. `suggestViewMode` leaves a Unity project in its
    Unity pane, which hides the solution Unity generates.
  - **Keymap.** The Visual Studio keymap's Alt+Down (Move Line Down) took the key from
    upstream's search-history actions in the Find popup. Those actions keep it, because
    they don't clash in practice: Move Line Down acts in the editor and history only
    in a search field.
  - The new Polish and Portuguese catalogues drop upstream's .NET run configuration
    strings and use the Bayonet names, as fr/ko/zh do.
- **2026-11-06, `88ecaf9c` (v0.1.38 and 79 commits).**
  - **csharp-ls decompiled definitions** (`c3e8f20f`). Upstream's `csharp:` metadata
    documents are a branch beside our `roslyn-source-generated` one in `virtual_document`.
    Roslyn's own decompiled files stay on disk under `MetadataAsSource` (read-only through
    `is_metadata_source`), so this only changes the csharp-ls fallback. The banner keeps
    our generated-source text and adds upstream's "Decompiled assembly" text for `csharp:`.
  - **References scope** (`a6d17543`). `lspReferences` gained `libraries` and a partial
    results channel. Change Signature and continuous testing pass `false` and an unused
    channel; `libraries` only narrows Java searches.
  - **Update source.** Upstream's GitHub/CNB update source setting stays removed, as our
    own update feed replaced it (`0c64b515`); its strings are dropped from en/zh.
  - `release.yml` stays deleted (its Windows portable zip, #1864, is not in our release
    kit). `tauri.conf.json`, the README and the Korean Bayonet title stay ours.

## What was renamed

User-visible names only:

- `src-tauri/tauri.conf.json`: product name, window title, bundle identifier
  (`io.github.skuirrels.bayonet`), main binary (`bayonet`), descriptions.
- Menus (`src-tauri/src/menu.rs`), locale strings (`src/platform/locales/`), title bar,
  welcome screen, memory indicator, settings import/export dialogs, splash screen.
- `README.md`, `NOTICE`.

A new bundle identifier means Bayonet keeps its own settings and caches, separate
from an installed OpenIDEA.

The port was first called DotnetIDEA and was renamed to Bayonet on 2026-11-05. That
rename covered the names above plus the port's own internals: the `bayonet-monitor-*`
and `bayonet-coverage` temporary files, the test runner's LSP `clientInfo`, and the icon
sources. The bundle identifier changed from `io.github.skuirrels.dotnetidea`, so Bayonet
starts with fresh settings; DotnetIDEA's are not migrated.

## What deliberately kept the OpenIDEA name

Renaming these would conflict with nearly every upstream merge, and none is visible to users:

- Rust crate and module names, including the `openidea` package in `src-tauri`.
- `OPENIDEA_*` environment variables, `openidea-*` temporary file prefixes, thread names.
- The per-project folder `.openidea/` (`crates/ide-core/src/project_dir.rs` and about 80
  other references). Projects can be opened by both IDEs. Spec section 4.2 planned a
  rename to `.bayonet/`; that is deferred until the port has diverged enough to
  justify the merge cost.
- The plugin ABI package `openidea:plugin@0.1.0` (`wit/plugin.wit`) and protocol
  extensions such as `_openidea/steer`.
- `openidea-server`, the headless backend binary used for remote development.
- CSS class names and SVG source comments.

## Disabled

- **The CNB update mirror.** OpenIDEA fell back to its CNB mirror (and offered it for the
  Chinese UI); Bayonet has one update source, its release site, so the "Update source"
  setting is gone.

## Releases

Releases use the release kit shared with Tessera and the other apps (`scripts/release-kit/`,
synced from `skuirrels/release-kit`): `make release` on a Mac and on a Windows PC builds the
installers locally and publishes them to https://skuirrels.github.io/bayonet-releases/ with
signed per-platform update feeds. `RELEASING.md` has the steps, the settings and the key.

## Still to do

- The app icon and splash mark are Bayonet's own (a "B" on a violet frame, source in
  `src-tauri/icons/source/bayonet.svg`; regenerate with `pnpm tauri icon` on that file,
  then delete the Android, iOS and 64x64 outputs it adds). The "B" is stroked at the same
  weight as the frame's lettering. The wordmarks (`bayonet-wordmark.svg`, `-dark.svg`)
  set "Bayonet" in Inter Bold at 66 px, outlined with fontTools from the variable Inter
  the UI already ships (`@fontsource-variable/inter`, weight 700). The Windows
  icons come from `bayonet-windows.svg` (a larger mark) via `pnpm gen:windows-icons`.
- Remote development still downloads `openidea-server` from OpenIDEA's GitHub releases of
  the same version (`crates/ide-tools/src/server.rs`). The release kit doesn't publish a
  Bayonet server yet: its Linux hosts need a musl build, which the Mac can't make.
- Unity and Mono debugging (DBG-7) is out of scope for now: it needs a Mono soft-debugger
  adapter, and nothing on the development machine can check it.

## Changed defaults

- **Keymap.** The default preset is "Visual Studio" (`visualstudio-windows`, or
  `visualstudio-macos` with Ctrl read as ⌘), in `src/platform/keymaps/`. It is the
  Windows or macOS IDEA keymap with Visual Studio's published default shortcuts on top;
  a key Visual Studio assigns is removed from every other action. The IDEA presets are
  still offered.
- **Project view.** A "Solution" pane lists the `.slnx`, `.sln` or `.slnf` at the
  project root (`crates/ide-dotnet`, `src/features/dotnet`). It opens first when the user
  has never chosen a pane, and shows the plain project when there is no solution.
  Projects leave out `bin` and `obj` unless hidden files are shown.
- **Opening a solution (SLN-1).** File › Open Solution… (also on the title bar's project
  popup), a `.sln`, `.slnx` or `.slnf` dropped on the Welcome screen, or one passed on the
  command line or with Finder's Open With opens the solution's folder as the project. Once the
  project is open (and trusted), the window keeps that choice in `.openidea/solution.local`
  (git-ignored), so the Solution pane, Build, Run, Unit Tests and NuGet use that solution, also
  after a restart or from Open Recent. A window that already has the folder open switches to
  it. From Finder or the command line, a solution in a subfolder of a project open in a window
  opens there as a file. Without a choice, the root's `.slnx` comes first, then `.sln`, then
  `.slnf`. In a multi-project workspace the Solution pane still reads the workspace root only.

## .NET inner loop (phase 2)

- **Build.** `BuildSystem::Dotnet` in `crates/ide-build/src/compile/` builds the root
  solution (the opened one, else `.slnx` first) or, for Build Module, the file's project, with `dotnet build`
  (`--no-incremental` for Rebuild, `publish -c Release` for Build Artifacts). MSBuild
  messages become Problems and editor diagnostics. The `dotnet` host comes from
  Settings › .NET when one is set.
- **Run configurations.** `crates/ide-run/src/detect.rs` offers `dotnet run` per executable
  project, one per `Project` launch profile in `Properties/launchSettings.json`, and
  `dotnet test` when the solution has test projects. Edit Configurations has a ".NET
  Project" type (`src/features/dotnet/runType.ts`).
- **Debugging.** Debugging a `dotnet run` configuration sends a `dotnet` block to
  netcoredbg's launch (`crates/ide-dap/src/dotnet.rs`): the project is built, MSBuild
  gives `TargetPath` and the run settings, and the launch profile's arguments,
  environment and URLs are applied as `dotnet run` would. Exception breakpoints use the
  filters netcoredbg advertises.
- **Target framework.** For a file in a multi-targeting project the status bar shows its
  target framework; choosing another sends Roslyn's `_vs_projectContext` with every
  request for that file, so `#if` regions, completion and diagnostics follow it.

## NuGet (phase 3)

- The NuGet tool window (bottom stripe, shown when the project has a solution) searches the package sources, lists every project's packages and installs, updates or removes them per project or across the solution. **NuGet › Manage NuGet Packages…** opens it.
- It runs the SDK's own `dotnet package search / list / add / remove` (`crates/ide-dotnet/src/nuget.rs`) instead of a NuGet client in the sidecar. That way `nuget.config`, package source mapping, credential providers (NUG-3) and central package management behave exactly as on the command line. Installing into a CPM project writes the version to `Directory.Packages.props` (NUG-2). Tests and solution edits likewise use the SDK (MTP server mode, `dotnet sln`, `dotnet reference`), so phase 3 needed no sidecar.
- **Check for Updates** runs `dotnet package list --outdated`, which needs the sources, so it only runs when asked.
- Removing a package keeps its `PackageVersion` in `Directory.Packages.props`, as the SDK does.
- Vulnerable and deprecated packages (NUG-4): each time the packages load, `dotnet package list --vulnerable --include-transitive` and `--deprecated` run in the background (`nuget_audit`). Findings become warnings on the `Include` of the `PackageReference` in the project file. Packages that come in under another package, or are referenced from an imported file, get theirs on the `Project` element. The warnings use NuGet's audit codes (NU1901 to NU1904 by severity), and deprecated packages are struck through. They show in the editor and in Problems without the file open. The window badges such packages, lists their advisories and alternative, and has a **Vulnerable Transitive Packages** section. Installing a patched version directly is the fix there. When the sources cannot be reached, the last results stay.

## User secrets (PLT-4)

- **Manage User Secrets** (project context menu, or the palette for the active file's project) opens the project's `secrets.json` in the editor. The file lives where `Microsoft.Extensions.Configuration.UserSecrets` reads it: `~/.microsoft/usersecrets/<id>/` on macOS and Linux, `%APPDATA%\Microsoft\UserSecrets\<id>\` on Windows (`crates/ide-dotnet/src/secrets.rs`).
- The id comes from the project file, else from MSBuild (`-getProperty:UserSecretsId`, which covers `Directory.Build.props`). A project without one gets one from `dotnet user-secrets init`, then an empty `{}` file. An id that would leave the secrets folder is refused.

## Unit tests (phase 3)

- The Unit Tests tool window (bottom stripe) lists every test of the solution's test projects by project, namespace, class and method. Data-driven rows are grouped under their method. It can build and discover, run all, run the selection, stop, and filter by name or to failed tests. The selected test shows its message, expected and actual values, a stack trace whose frames open the source, and its output.
- Tests are driven through Microsoft.Testing.Platform's JSON-RPC server mode (`crates/ide-dotnet/src/testing.rs`). The host listens on a local port and starts `dotnet exec <TargetPath> --server --client-host 127.0.0.1 --client-port N`. It sends `initialize`, then `testing/discoverTests` or `testing/runTests` (with the selected uids), and folds the `testing/testUpdates/tests` notifications into the tree. This gives each test's type, method, file and line, which `dotnet test`'s console output does not. NUnit reports no file, so its locations are found in the project's sources.
- C# (and VB, F#) editors get gutter run icons on test methods and classes from the discovered locations, coloured by the last result. Clicking one runs just those tests.
- Not yet: VSTest-only test projects (they show "only Microsoft.Testing.Platform test projects are supported so far"), and continuous testing (TST-3).

## Solution editing (phase 3)

- **New › Project…** (on a folder, or **Add › New Project…** on the solution or a solution folder) lists every installed `dotnet new` project template, custom ones included, read from the template engine's cache (`crates/ide-dotnet/src/templates.rs`). It shows each template's options, runs `dotnet new`, adds the project to the solution (in the chosen solution folder) and opens it (SLN-7).
- **Add › Existing Project…**, **Remove from Solution** and **Add Project Reference…** run `dotnet sln add/remove` and `dotnet reference add/remove`, so the `.sln`/`.slnx` and project files keep their formatting (SLN-4).
- Group nodes in the project view (the solution and its solution folders) get a context menu from the new `projectViewGroup` menu id.
- **Convert Solution to .slnx** (on a `.sln` solution node) runs `dotnet sln migrate`, which writes the `.slnx` next to the `.sln` and keeps the `.sln`; the Solution Explorer prefers the `.slnx` from then on (SLN-6).
- **MSBuild navigation (SLN-5).** In project, `.props` and `.targets` files, Go to Declaration follows a version-less `PackageReference` to its `PackageVersion` in the nearest `Directory.Packages.props`, a `$(Property)` to where the file or a `Directory.Build.props/targets` above it sets it, and `Import` / `ProjectReference` paths to their files (`src/features/dotnet/msbuild/navigation.ts`). Properties set elsewhere (SDK targets, imported files) are not followed yet.
- **MSBuild completion and targets (CI-18).** In the same files, completion offers properties inside a `PropertyGroup`, item types inside an `ItemGroup` (inserting `Include=""`), tasks inside a `Target`, top-level elements, `$(…)` properties, `@(…)` item types and target names in `DependsOnTargets`, `AfterTargets`, `BeforeTargets` and `Targets`, with the names the file defines (`src/features/dotnet/msbuild/completion.ts`). Go to Declaration on a target name in those attributes looks in the file, `Directory.Build.props/targets`, the files it imports, then the .NET SDK the project uses (`<dotnet>/sdk/<version>`, `Microsoft.Common.CurrentVersion.targets` first; `crates/ide-dotnet/src/msbuild.rs`).

## Roslyn features (phase 4)

- Roslyn's options are set in `languages/csharp.toml` under `[language-servers.settings."<language>|<group>"]`, the names Roslyn asks for in `workspace/configuration` (`csharp|inlay_hints.dotnet_enable_inlay_hints_for_parameters`, `navigation.dotnet_navigate_to_decompiled_sources`, …).
- **Inlay hints (CI-9).** Parameter names for literals, indexers and object creation, and types for `var`, lambdas and implicit `new`, with Roslyn's suppressions for names that match. The editor's Inlay Hints setting turns them off.
- **Referenced assemblies (CI-10).** Go to Declaration into a package or the BCL opens Source Link sources when the package has them, else code decompiled with ICSharpCode.Decompiler. Roslyn writes these under `<temp>/MetadataAsSource/`; such files open read-only.
- **Solution-wide analysis (CI-7).** Analyser and compiler diagnostics run on the whole solution (`fullSolution`). Servers marked `workspace-diagnostics` in their TOML are pulled with `workspace/diagnostic` once they send a refresh. Roslyn holds a workspace pull open until something changes, so there is one pull at a time per server, sent with the previous result ids and sent again as soon as it answers (`crates/ide-lsp/src/pull.rs`). Problems in files that are not open appear in the Problems view and clear when fixed. Servers marked `pull-diagnostics` are now told the client supports pull diagnostics: Roslyn only answers workspace pulls and sends refreshes to such clients.
- **Reformat (CI-8).** Reformat Code and reformat on save use Roslyn's formatter. Reformatting now sends the Code Style or `.editorconfig` indentation, not the indentation detected in the file (which IDEA also uses for typing only), so the result matches `dotnet format`. Indent detection also skips preprocessor lines at column 0, which made C# files with `#if` blocks look 8-space indented. Optimise imports and cleanup on save ask for `source.organizeImports` / `source.fixAll` code actions; whether Roslyn offers them is not checked yet.
- **Source generators (CI-17).** A C# or VB project in the Solution Explorer starts with **Dependencies › Analyzers**: one folder per analyser assembly, and below a generator its files (`<assembly>/<generator type>/<hint name>`), read-only. Roslyn keeps generated sources in memory, so the IDE compiles the project once more with `EmitCompilerGeneratedFiles` into its build cache (`crates/ide-dotnet/src/generators.rs`, `src/features/dotnet/generated.ts`). An import passed as `CustomAfterMicrosoftCommonTargets` and conditioned on the project's path moves only that project's intermediate folder there, so its `obj` and `bin` (and those of the projects it references) are not touched. The project's own `CustomAfterMicrosoftCommonTargets` is evaluated first and imported from that file, since the global property would replace it. This runs when the node is first expanded, and again shortly after the project's sources change while it is expanded (build output in `bin` and `obj` does not count; a change during a compile runs it again afterwards). A compile that fails without output, such as while code is being typed, keeps the earlier listing; one that fails because referenced projects are not built offers to build. Go to Declaration into generated code opens Roslyn's own copy (`roslyn-source-generated://`, read with `sourceGeneratedDocument/_roslyn_getText` and never cached), with a banner naming the generator. Packages, project references and frameworks are not listed under Dependencies yet.
- **.NET Aspire (BLD-5).** A solution with an AppHost (`Aspire.AppHost.Sdk`) gets an **Aspire** tool window. It talks to the AppHost through the Aspire CLI, which needs no workload (`~/.aspire/bin`, a global tool or the search path; the window offers `dotnet tool install --global Aspire.Cli` when it is missing). Whether the AppHost runs comes from polling `aspire ps` every two seconds, so an AppHost started from a terminal shows too; while it runs, `aspire describe --follow` streams its resources: name, type, state and health, endpoints (opened in the browser) and the commands each offers (start, stop, restart, rebuild and custom ones, through `aspire resource`). Selecting a resource follows its console with `aspire logs --follow` (history first, ANSI colours kept, the last 5,000 lines). **Run** starts the AppHost's detected `dotnet run` configuration, so it also shows in the Run window; **Stop** runs `aspire stop`; the globe opens the dashboard with its login token (`crates/ide-dotnet/src/aspire.rs`, `crates/ide-host/src/ops/aspire.rs`, `src/features/dotnet/aspire/`). Resources can be debugged too (DBG-5, below).
- **Debugging tests and Aspire resources (DBG-5).** **Debug** in the Unit Tests window (toolbar, context menu) and on a test's gutter icon builds and runs the tests with the test app started waiting for a debugger (`TESTINGPLATFORM_WAIT_ATTACH_DEBUGGER`); the backend reports the process it started (`TestEvent::Debug`) and the window attaches netcoredbg to it, after which the run goes on and breakpoints in tests hit. A debug run collects no coverage. The Aspire window attaches the debugger to a running project resource (its `executable.pid`). Both need netcoredbg; with no debugger attached, the test app waits up to ten minutes.
- **C# refactorings (CI-6).** Roslyn's refactorings appear as IDEA's: Extract Method (and local function), Introduce Variable, Constant and Parameter, Extract Interface and Extract Superclass (new Refactor menu entries), Inline and Move (Move type to file, which creates the file); Rename and Safe Delete are the IDE's own. Roslyn groups some of them (Introduce Parameter "and update call sites directly", "into new overload", …) under one action whose command, `roslyn.client.nestedCodeAction`, asks the client for a submenu; the backend offers each member as its own action titled with its path, so they resolve like any other (`crates/ide-lsp/src/convert.rs`). `roslyn_refactorings` in `crates/ide-lsp/tests/real_servers.rs` runs each on `fixtures/dotnet/src/Fixture.Core/Pricing.cs` and checks the result. Roslyn offers Change Signature only with Visual Studio's dialog, not over LSP, so the IDE does it for C# itself: `registerRefactoringFallback` (`src/editor/refactor/refactorThis.ts`) hands `refactor.changeSignature` in C# files to `src/features/dotnet/signature/`, which also lists it in Refactor This. The caret's callee (or the declaration around the caret) is read with tree-sitter (`signature_at`, `crates/ide-analysis/src/signature.rs`), the dialog reorders, removes and adds parameters (a new one gets a value for callers, or is omitted when it has a default), and Roslyn's references, declaration included, give the sites. Each site is rewritten on its own: arguments follow their parameters, nested calls are rewritten inside the outer call, an omitted optional argument turns what follows into named arguments, an extension method's receiver stays put, and declarations Roslyn cascades to (the interface member, overrides) change with it. Method groups and lists that cannot be mapped safely are left alone and listed in a warning. Renaming a parameter is left to Rename, so the body never refers to a name that is gone. `roslyn_change_signature` in `crates/ide-lsp/tests/real_servers.rs` runs it against Roslyn's references across two files.
- **Continuous testing (TST-3).** The ↻ button in the Unit Tests toolbar reruns the tests affected by a change, on save or after a successful build (remembered per project; off by default). Each open C#, F# or VB file's members (from its outline) are compared with their text when it was opened or last saved; for a changed member, the tests whose method it is, or that reach it through Roslyn's find references up to three calls away (40 searches at most), run. Before tests are discovered, a change runs them all once. Tests are matched by their source line, so a change reaches only tests the Unit Tests window has located (`src/features/dotnet/tests/continuous.ts`). It does not use coverage data, so calls through interfaces, reflection or dependency injection are only followed where find references sees them.
- **F# (CI-14).** fsautocomplete (`languages/fsharp.toml`) gives F# completion, diagnostics and hover; an ignored end-to-end test in `crates/ide-lsp/tests/real_servers.rs` (`OPENIDEA_TEST_FSAC`) checks them on the fixture's F# project. F# has no source generators, so F# projects have no Dependencies node.
- The fixture now has a source generator, `src/Fixture.Generators` (an incremental generator collecting minimal API `WithName` names into `EndpointNames`), referenced by `Fixture.Api` as an analyser, which also uses `[GeneratedRegex]`.

## Hot Reload (phase 4)

- A .NET Project run configuration has a **Hot Reload (dotnet watch)** option (BLD-4). The command becomes `dotnet watch --non-interactive run …`: code edits are applied to the running app, and edits Hot Reload cannot apply restart it instead of waiting for an answer. Debugging such a configuration runs it under netcoredbg without Hot Reload.

## Test coverage (phase 4)

- **Run All Tests with Coverage** (Unit Tests toolbar, the selection's context menu, and the **Run All Tests in Solution with Coverage** action) runs the tests with Microsoft Code Coverage and shows the result as a "Unit Tests" suite in the Coverage tool window, the editor gutter and the project tree (TST-4).
- It needs no installed tool: test projects that reference `Microsoft.Testing.Extensions.CodeCoverage` (the MSTest metapackage brings it in; detected from `obj/project.assets.json`) get `--coverage --coverage-output-format cobertura` and write one Cobertura report each, which `crates/ide-coverage/src/cobertura.rs` merges. Projects without the extension still run, and a notification offers to add the package to them.
- The coverage extension makes the test app a test host controller that relaunches itself: the controller connects to the IDE first and reports `supportsDiscovery: false`. The session keeps that connection open and drives the test host's connection, which comes second.
- Not yet: coverage for VSTest-only projects, and coverlet reports from run configurations.

## Attach to Process (phase 4)

- **Attach to Process…** lists .NET processes first, under ".NET (netcoredbg)" (DBG-4). A process counts as .NET when the runtime has opened its diagnostics IPC endpoint (`dotnet-diagnostic-<pid>-…-socket` in `$TMPDIR`, a `dotnet-diagnostic-<pid>` pipe on Windows), so apphosts, `dotnet exec` apps and tools are all found; `dotnet <App>.dll` hosts show the app's name. Attaching sends netcoredbg `attach` with `processId` and Just My Code from the debugger settings.
- Checked against netcoredbg 3.2.0 (installed by the IDE's tool installer) with ignored tests in `crates/ide-dap/tests/real_adapters.rs`. `netcoredbg_launches_a_project_and_reads_threads_and_collections` builds a console project through the `dotnet` launch block and stops at a breakpoint. `netcoredbg_attaches_to_a_running_process` attaches to `dotnet <app>.dll` and hits a breakpoint set after attaching. `netcoredbg_debugs_a_waiting_test_app` starts an MSTest (Microsoft.Testing.Platform) app with `TESTINGPLATFORM_WAIT_ATTACH_DEBUGGER=1`, attaches and stops in the test (DBG-5). The Unit Tests window and the Aspire window that start these were not driven in the app.
- Driven in the dev app (2026-11-06): **Attach to Process…** opened its popup with the .NET processes first (14 under ".NET (netcoredbg)": Roslyn, MSBuild nodes, the user's own apps and a Docker container's `dotnet` process), filterable by name, PID or command line.
- netcoredbg matches breakpoints by the path the compiler recorded, which is the real path. A project opened through a symlinked folder (on macOS, `/var` is `/private/var`) never binds its breakpoints.

## Parallel Stacks (DBG-6)

- The debugger tool window has a **Parallel Stacks** tab: while the program is suspended, every thread's call stack is merged into one graph (`src/features/debugger/parallel/`). Threads that share callers share boxes, entry points at the bottom and each thread's own frames above, and a box's header counts its threads (the tooltip names them). Clicking a frame selects that thread and frame, and the current thread's path is highlighted. Frames of one method in one file merge even when threads are on different lines. Up to 64 threads are read.
- It uses only DAP `threads` and `stackTrace`, so it works with any adapter. Checked against Xcode's lldb-dap with a four-thread C program (`lldb_dap_reports_every_threads_stack` in `crates/ide-dap/tests/real_adapters.rs`). That check found that lldb-dap 21 answers the first `threads` request after `configurationDone` with the thread list it cached at launch, so the debug manager now asks for the threads once after configuration, as VS Code does.
- **Set Next Statement** (Run menu; Ctrl+Shift+F10 in the Visual Studio keymap) moves the paused thread's execution point to the caret's line without running the code in between: DAP `gotoTargets` for the line, then `goto` to the first target. It is offered only when the adapter reports `supportsGotoTargetsRequest` (debugpy, CodeLLDB and vsdbg do; lldb-dap 21 and netcoredbg 3.2 do not). Checked with the mock adapter only, since no installed adapter supports it. Dragging the execution point is not done.
- **View as Table** (the variables context menu, on any expandable value) opens the collection visualiser (`src/features/debugger/grid/`): the collection's elements as rows and their members as columns, sortable by any column (numbers numerically, missing cells last). Scalar elements go in a Value column. Elements are the children named `[0]` or `0`. Range groups (`[0 … 99]`, from js-debug and debugpy) and a Results View are opened to reach them. netcoredbg has no type proxies, so a `List<T>` appears as its fields: the table reads its `_items` array up to `_size`. For other .NET collections without elements it evaluates `(expression).ToArray()`, which netcoredbg can call on instance methods (not on LINQ's static `Enumerable.ToArray`); a `Dictionary` therefore shows nothing yet. Adapter groupings such as Raw View, Static members and `[[Prototype]]` are not columns. Up to 500 elements are read, and members are fetched 16 elements at a time. At a later stop, a value that has an expression is read again in the selected frame. Otherwise the table is dimmed and marked as from an earlier stop. Checked with unit and component tests, and netcoredbg's view of a `List<T>` with the real adapter test above.
- With netcoredbg, the Parallel Stacks data (four parked worker threads, each with its own stack) is checked by `netcoredbg_launches_a_project_and_reads_threads_and_collections`.

## Visual Studio `.http` files (phase 4)

- The HTTP client (IntelliJ syntax) also runs Visual Studio's `.http` files (PLT-3, `crates/ide-http/src/env.rs`):
  - `http-client.env.json.user` is read as the private environment file next to `http-client.private.env.json` (the IntelliJ one wins for a name both set).
  - Dynamic variables `$guid`, `$randomInt min max` (max exclusive), `$timestamp [offset unit]`, `$datetime` and `$localDatetime` with `rfc1123`, `iso8601` or a quoted .NET format and an optional offset (`1 d`, `-2 h`; units y, M, w, d, h, m, s, ms), `$processEnv NAME`, and `$dotenv NAME` from the nearest `.env` at or above the request file.
  - Request variables: `{{login.response.body.$.token}}`, `{{login.response.body.*}}`, `{{login.response.headers.Location}}` and the `request.` equivalents read the last successful response of the request named `# @name login` in the same file (from the response history). The JSONPath subset is `$`, `.key`, `[n]` and `['key']`; XPath is not supported.

## Entity Framework Core (phase 4)

- Projects that use EF Core (`Microsoft.EntityFrameworkCore` in their restore output) get an **Entity Framework Core** submenu in the Solution Explorer: Add Migration…, Remove Last Migration, Update Database… (to the latest, any migration, or `0`), Generate SQL Script… (idempotent) and Scaffold DbContext… (connection string, provider, output folder) (PLT-2, `crates/ide-dotnet/src/ef.rs`, `src/features/dotnet/ef/ef.ts`).
- Each runs `dotnet ef … --project <project> --startup-project <startup>` in the Run tool window. The startup project is the project itself when it can start, else the first executable project in the solution that references it.
- The tool is `dotnet ef` when that works in the project folder, else the global tool's executable `~/.dotnet/tools/dotnet-ef` (a local tool manifest that was never restored hides the global tool from `dotnet ef`), else a notification offers `dotnet tool install --global dotnet-ef`.
- Migrations for Update Database come from the migration file names (no build).

## Run in Container (PLT-5)

- **Run in Container** (a project's context menu, or Find Action) asks for an image name. It then runs one shell line in the Run window: `dotnet publish -t:PublishContainer` (the SDK's container support, no Dockerfile) builds `<name>:dev`, and `docker run --rm -i` runs it (`crates/ide-dotnet/src/publish.rs`).
- For a web project, the first `http://` port of its launch profiles (else 8080) maps to 8080, the port ASP.NET Core listens on in its images. Stopping the run stops the container, because `docker run` forwards the signal.
- **Debugging in a container** (DBG-8): Attach to Process also lists the .NET processes of running Docker containers, named by the container (`docker exec` runs a POSIX `sh` loop over the runtime's diagnostics sockets in `/tmp`, so images without `ps` work). Attaching copies a Linux netcoredbg for the container's architecture (`uname -m`, downloaded once into `tools-linux-<arch>` in the app data) to `/tmp/openidea-netcoredbg` unless it is already there, then starts it through `docker exec -i <container>` as a `pipeTransport` (the C# extension's launch.json shape: `pipeProgram`, `pipeArgs`, `debuggerPath`, `debuggerArgs`, `pipeCwd`) (`crates/ide-dap/src/container.rs`, `prepare_container` in `crates/ide-host/src/ops/debug.rs`).
- Breakpoints bind only when the paths in the PDB match the host's sources: run what the host built (Run in Container's `PublishContainer`, or a bind mount of the build output). An image whose Dockerfile builds the code inside the image records `/src/…` paths, and its breakpoints do not bind; source path mapping is not done.
- `pipeTransport` in a launch configuration also works for SSH (`"pipeProgram": "ssh", "pipeArgs": ["box"]`), but there is no UI for it, and netcoredbg must already be on the remote machine.
- Checked by `attaches_to_a_dotnet_app_in_a_container` (ignored; `crates/ide-host/tests/container_debug.rs`): an app built on the host runs in `mcr.microsoft.com/dotnet/aspnet:10.0`, is listed, attached to through Docker Desktop and stops at a breakpoint. The popup was not driven in the app.

## Language server crashes (NFR-4)

- The .NET processes are services: the text of open files lives in the Rust core, so editing and saving go on while Roslyn (or fsautocomplete, lemminx) is down. A crashed server restarts on its own with backoff (a server that crashes repeatedly waits for **Restart** in the language server popup). Once it is running again, every open document is opened on it again with the editor's current text, so unsaved edits are not lost to it.
- While a file's server is crashed or restarting after a crash, a banner above the editor says so and that editing and saving still work; while crashed it offers **Restart** (`src/features/code-insight/degradedBanner.ts`). The banner goes once the server is running, and does not show when another running server covers the file's language.
- Checked by `roslyn_restarts_after_a_crash_with_the_unsaved_text` (ignored; `crates/ide-lsp/tests/real_servers.rs`). It kills Roslyn, edits the file while Roslyn is down, waits for the restart, and hovers a variable that exists only in the unsaved text. The banner was also driven in the dev app (2026-11-06, through the MCP bridge, on a copy of `fixtures/dotnet` with `Greeter.cs` open): after `kill -9` of Roslyn the warning banner with **Restart** showed within 50 ms and the status dot turned red; Roslyn restarted on its own and the banner went, with code lens and inlay hints back. Roslyn's default target framework for the file was netstandard2.0 before the crash and net10.0 after it: nothing had been chosen, and a chosen context is kept and sent again.

## Telemetry and licences (NFR-6, NFR-7)

- The IDE sends no telemetry. Every SDK command it runs itself sets `DOTNET_CLI_TELEMETRY_OPTOUT=1` (and `TESTINGPLATFORM_TELEMETRY_OPTOUT`, `ASPIRE_CLI_TELEMETRY_OPTOUT` where they apply), as do the Roslyn language server's environment and the Avalonia previewer's build and host.
- `THIRD_PARTY_LICENSES.md` lists the .NET tools that are downloaded on request (Roslyn, csharp-ls, FsAutoComplete, LemMinX, netcoredbg, the diagnostics tools); none is bundled. The crates this port added (`png`, `base64`) are MIT or Apache-2.0, within `deny.toml`'s allow list.

## Typing latency (NFR-3)

- `pnpm perf --only typing,typingLarge,keyRepeat,popups` (production bundle in Chromium, CPU 1x, 3 runs, 2026-11-04): typing p95 16.7 ms (max 17.3), a large file p95 16.3 ms, key repeat p95 16.6 ms with 1.1 % missed frames; Find Action and Search Everywhere open in 17 and 14 ms. All are within `perf/budgets.json`. The harness reports p50, p95 and max, not the spec's p99; at the 60 Hz frame (16.7 ms) the p95 is one frame.
- **Bundle budgets raised** (2026-11-06): `perf/budgets.json` now allows 1550 KB of initial JS and 87 KB of CSS (were 850 and 60), just above what `pnpm perf` measures after the upstream merge at `0075cee3`, 1518 KB and 85.1 KB, so growth from here still fails. Meeting the old budgets would mean splitting upstream's workbench, which would make every upstream merge harder. Before the merge: the same run failed the budgets at initial JS 1446 KB and CSS 83.6 KB. Upstream already fails them: the merged upstream base (48e23c01) measures 1401 KB and 82.1 KB, so this port adds 45 KB (3 %) and 1.5 KB. CI does not run `pnpm perf`. The size is spread across the workbench (react-dom, the LSP editor code, GitHub, Run, VCS, Search …); the Avalonia previewer is loaded on demand.
- **Completion popup**: the new `completion` scenario times Ctrl+Space to the painted lookup: p50 55 ms, p95 59 to 67 ms across runs (budget 75 ms). About 50 ms of that is CodeMirror's fixed wait before it queries the sources of an explicit completion. Roslyn's own answer is timed by `roslyn_completion_latency` (ignored, `crates/ide-lsp/tests/real_servers.rs`): member completion on the fixture after loading, p50 3.7 ms, p95 5.8 ms, max 11.3 ms over 40 requests at one position. Together that is well under NFR-3's 150 ms, for a small solution.
- **NFR-1, NFR-2: how to run.** `perf/compare/gen-dotnet-solution.sh <dir>` writes and builds a 100-project solution; `perf/compare/apps.toml` has `bayonet` (ready once Roslyn logs `Completed (re)load of N project(s)`) and `rider` (ready at `SolutionInitializerActivity - Ready for work`). A run needs a release Bayonet in `/Applications`, Rider set up as the harness README describes for IDEA, and `python perf/compare/bench.py <dir> --apps bayonet,rider`. Bayonet starts Roslyn only for an open C# file, and a file given on the command line opens in LightEdit, so open `Module100/Program.cs` by hand once in each IDE and quit with it open: both reopen it, and "ready" then includes its diagnostics, as NFR-1 asks.
- **NFR-1, NFR-2: results** (2026-11-06, M-series Mac, 14 cores, 24 GB, warm, Rider 2026.2.4, Roslyn 5.13.0-1.26514.10, medians). Memory is the summed macOS footprint (Activity Monitor's "Memory") of every process after 30 s idle.

  |                            | Bayonet         | Rider      |
  | -------------------------- | --------------- | ---------- |
  | Window with the solution   | 0.8 s           | 8.1 s      |
  | All 100 projects evaluated | 8.9 s           | 8.5 s      |
  | Open file's diagnostics    | 10.1 s          | not logged |
  | CPU settled                | 26.1 s          | 21.5 s     |
  | Memory after idle          | 791 MB          | 3712 MB    |
  | Idle CPU                   | 1.0 % of a core | 11.4 %     |
  | Caches on disk             | 8 MB            | 2173 MB    |

  NFR-1 passes: every project is evaluated at 8.9 s and Roslyn's report on the open file (`Module100/Program.cs`, no findings) reaches the editor at 10.1 s, against 15 s. That report is a debug log line (`ide_lsp::pull`), which the `bayonet` entry in `apps.toml` turns on through `RUST_LOG`. The CPU settles later because the NuGet tool window lists and audits the solution's packages (`dotnet package list`, about 15 s of MSBuild evaluation) alongside Roslyn's load. NFR-2 passes at 791 MB, against 1.5 GB, once idle; during the load the summed RSS peaks at 3.3 GB (it counts shared pages once per process, and includes the MSBuild nodes of the listing). Two fixes got it there: NuGet listings no longer leave a reusable MSBuild node per core running after they finish (1.66 GB, and they outlived the IDE by 15 minutes), and Roslyn runs with the workstation GC (`DOTNET_gcServer=0`, unless the environment sets it): the server GC it ships with held 1.9 GB here, and loading took no longer without it (completion p95 3.1 ms against 4.6 ms in `roslyn_completion_latency`). Rider's times are its fastest launch (its memory and idle CPU are medians): in eight of its ten launches its UI thread waited 46 to 237 s on macOS accessibility calls (`CAccessibility` in `idea.log`) before the window appeared, which points to an accessibility client on that machine rather than Rider.

## CI on this branch

- Not run on GitHub: the branch has not been pushed. Run locally on 2026-11-06 (macOS arm64, working tree at `0075cee3` plus the NFR-1/NFR-2 changes) instead. Passing: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (0 failures), the `.NET fixture` job (`dotnet test --solution DotnetFixture.slnx`: 7 of 7; `dotnet build DotnetFixture.sln`: 0 errors), `pnpm typecheck`, `pnpm format:check`, `pnpm knip`, `pnpm test` (7173 tests) and `pnpm build`.
- `pnpm lint` failed on one error in an uncommitted edit to `scripts/tauri.ts` from another session (a plain function named `useMacosDevBundle`, which the hooks rule takes for a hook); the committed file is clean.
- The ignored Roslyn tests (`cargo test -p ide-lsp --test real_servers -- --ignored roslyn`) pass one at a time (`--test-threads=1`): 8 of 8. In parallel, eight Roslyn servers start at once and `roslyn_completion_latency` (p95 over 90 ms) and `roslyn_refactorings` (no code action in time) fail. CI does not run them.
- `pnpm e2e` failed 19 of 148 before this run's fix: every spec presses IDEA's shortcuts, and the browser build behind them fell back to this port's default Visual Studio keymap (⌘W, ⌘R, ⌥F12, ⌘B, ⇧F6, ⌃R and ⌘F2 do other things there or nothing). `e2e/fixtures.ts` now applies the `intellij-macos` preset before each spec; with it, 148 of 148 pass. `src/platform/keymaps/visualstudio.test.ts` checks the Visual Studio presets themselves: each of its shortcuts reaches its action, is taken from the IDEA actions that had it (bar the deliberate Alt+Down share), and on macOS reads Ctrl as ⌘ without the system's keys.
- `cargo build --locked -p ide-host --bin openidea-server` builds natively and the binary reports `openidea-server 0.1.37 (protocol 2)`. `cargo deny check` passes (advisories, bans, licences, sources; duplicate-version warnings only), and so does `plugins/build.sh --check`. Not run locally: the musl server build (a Linux cross-build that needs the musl target and linker).

## Go to Everything (CI-11)

- Search Everywhere, Go to Class / Symbol / File and File Structure use the project's own tree-sitter index, which includes C# types and members. Results do not wait for Roslyn to load the solution. The test `csharp_symbols_are_found_quickly_in_a_large_solution` indexes 5,000 C# files and checks that lookups stay under 100 ms, even in a debug build.

## Hierarchies, structural search and the dependency diagram (CI-12, CI-13)

- Call hierarchy (incoming and outgoing) and type hierarchy (supertypes and subtypes) come from Roslyn over LSP through the shared hierarchy views. The ignored test `roslyn_hierarchies` checks all four against the real server. File structure is Roslyn's document outline.
- Structural search and replace runs ast-grep on the tree-sitter C# grammar, so `$x$ == null` finds null comparisons however they are formatted. Replacing with `$x$ is null` rewrites them (test `csharp_null_comparisons`).
- **Show Project Dependency Diagram** (Find Action, and the solution's context menu) opens a tab that draws the solution's projects and their project references in layers: a project sits above everything it references. References on a cycle are dashed. Clicking a project highlights what it references and what references it, and double-clicking opens its project file.
- Data-driven tests (TST-5): in the Unit Tests window, each `InlineData` / `TestCase` row is a node under its test method.

## Razor and Blazor (CI-15)

- `.razor` and `.cshtml` files are the `razor` language (`languages/razor.toml`). Roslyn's server hosts Razor itself (cohosting, `Microsoft.VisualStudioCode.RazorExtension.dll` ships in the `roslyn-language-server` tool), so the same instance as C# serves them: components and their parameters, directives, and the C# in `@code` blocks and expressions, with diagnostics and semantic tokens. The two languages must start Roslyn identically, which `razor_starts_the_same_roslyn_as_csharp` checks.
- Roslyn hands the HTML half of a Razor request back to the client: `razor/updateHtml` with the HTML projection, then the request itself with a `checksum`. No HTML server is attached to the projection, so these get empty answers (`crates/ide-lsp/src/handler.rs`); a `null` completion would drop Razor's own items too. The editor's HTML mode gives tag and attribute completion for the markup.
- `roslyn_razor_completion` in `crates/ide-lsp/tests/real_servers.rs` creates a Blazor app with `dotnet new blazor` and checks that `<Cou` completes the app's components and the framework's.

## XAML bindings (CI-16)

- In `.xaml` and `.axaml` files (WPF, Avalonia, MAUI, WinUI), binding paths complete with the members of the binding's data type, and Go to Declaration opens the C# member or type a binding path, an element tag (`<local:Card>`) or a type attribute (`x:DataType`, `DataType`, `x:Class`, `TargetType`) names (`src/features/dotnet/xaml/`).
- The data type is the nearest `x:DataType`, a WPF `d:DataContext="{d:DesignInstance …}"`, or a `DataTemplate`'s `DataType`; `x:Bind` binds to the `x:Class`. `xmlns` prefixes map to CLR namespaces through `clr-namespace:` and `using:`. Each path segment is followed into its member's type, and into a collection's element type after an indexer (`Items[0].Title`).
- The members come from the project's C# sources, not a compiler: the index finds the type's files, tree-sitter reads every partial part (`crates/ide-analysis/src/csharp_types.rs`, `csharp_type_members` in `crates/ide-host/src/ops/analysis.rs`), and base types declared in the project add theirs. Public properties count, and so does what CommunityToolkit.Mvvm generates: `[ObservableProperty]` fields as properties (`_name` is `Name`) and `[RelayCommand]` methods as `…Command`. Members of types from referenced assemblies (a base `ObservableObject`, a framework type) are not listed.
- **Avalonia previewer.** An `.axaml` file opens as Editor and Preview, like Markdown, with Editor Only and Preview Only in the tab strip (`src/features/dotnet/xaml/preview/`). The preview builds the file's project once, then runs Avalonia's own designer host (`Avalonia.Designer.HostApp`, found through MSBuild's `AvaloniaPreviewerNetCoreToolPath`) on the project's runtime config and deps file. The host connects back to a loopback port and speaks Avalonia's remote protocol: a little-endian length, the message's GUID and a BSON body (`crates/ide-dotnet/src/avalonia.rs`, a small BSON codec of its own). Every edit sends the editor's text (debounced 250 ms). A XAML error keeps the last good frame dimmed and shows the message with a link to its line; the compiler's aggregate errors are split into their distinct parts. Frames arrive as RGBA at the window's pixel ratio and are passed on as PNGs. Zoom out, actual size, zoom in, and Build and Restart (for code-behind or view model changes) are in the preview's toolbar. Hiding the preview stops the host.
- Checked against Avalonia 12.1.5 with `fixtures/avalonia/AvaloniaFixture` (a window using `Design.DataContext`): the ignored test `renders_xaml_and_reports_xaml_errors` in `crates/ide-dotnet/tests/avalonia_preview.rs` gets an 800×480 frame at scale 2, a misspelt element's error on its line, and a new frame after the fix. WPF and MAUI `.xaml` files have no previewer.
- Not done: element and attribute completion beyond what LemMinX gives for XML.

## Publish (BLD-6)

- **Publish…** (on a project's context menu, or in Find Action for the active file's project) runs `dotnet publish` in the Run tool window (`crates/ide-dotnet/src/publish.rs`). It offers three kinds of target:
  - One of the project's `Properties/PublishProfiles/*.pubxml` profiles (`-p:PublishProfile`, with the profile's configuration).
  - A folder (`-c Release -o <folder>`).
  - A container image. This uses the SDK's container support (`-t:PublishContainer -p:ContainerRepository=<name>`), so it needs no Dockerfile, and the image goes into the local Docker or Podman.
- The list shows each profile's method and destination, read from the `.pubxml`. Profiles that publish to a remote host (MSDeploy, ZipDeploy) run as the SDK runs them, using the credentials in the matching `.pubxml.user`.

## Build progress and binary logs (phase 4)

- A .NET build lists the solution's projects under the Build tool window's root, each with its state (building, built, failed, not built) and the files with messages below it. The status bar task shows the fraction of projects built (BLD-3, `crates/ide-build/src/compile/mod.rs`, `src/features/build`). A project counts as built when MSBuild prints its `Name -> output` line, and as failed when it has errors.
- Cancel kills the whole process group, MSBuild's worker nodes included; `dotnet_build_cancels_within_two_seconds` checks the acceptance against the installed SDK.
- **Settings › Languages › .NET › Write a binary log of every build** adds `-bl` to builds. The last 10 logs are kept in the project's build cache; the Build tool window's root links to the build's log, and **Show Last Binary Log** opens the newest.
- `.binlog` files open in a binary log viewer (`crates/ide-dotnet/src/binlog.rs`, `src/features/dotnet/binlog`): errors and warnings, then projects with their targets, tasks and the projects they built, each with its duration and its place on the build's timeline; a summary with the slowest targets; search over names, command lines and messages; skipped targets on request. The reader follows MSBuild's own format (versions 18 and later, .NET 8 SDK on) and skips records it does not need by their length.
- Projects show their own icons, a boxed C# or F#, distinct from source files.

## .NET diagnostics (phase 4)

- **View › Tool Windows › .NET Diagnostics** (DIAG, `crates/ide-dotnet/src/diagnostics.rs`, `crates/ide-host/src/ops/diagnostics.rs`, `src/features/dotnet/diagnostics`) works on a running .NET process picked at the top: this project's processes first, then other apps, then build and IDE tooling. Processes come from the runtime's diagnostic sockets; a socket that no longer accepts connections is left out, so stale ones from dead processes do not show (this also cleans up Attach to Process).
- **Counters**: live tiles (CPU, working set, GC heap, thread pool, exceptions) with sparklines, a large chart of the chosen counter with a crosshair read-out, and every counter in a filterable table. They stream from a `dotnet-monitor` sidecar on loopback at one-second intervals; it starts on first use and exits with the IDE (`--exit-on-stdin-disconnect`). **Record 10 s to CSV** runs `dotnet-counters collect` and shows the recording in place of the live view. **dotnet-monitor HTTP API** writes a `.http` file of the monitor's endpoints (processes, stacks, gcdump, dump, trace, logs, collection rules) for the process, to try in the HTTP client.
- **Threads**: `dotnet-stack report`, with identical stacks grouped and named after their innermost user-code frame. **SOS Shell on Live Process** opens `dotnet-debug attach` in the terminal.
- **Memory**: `dotnet-gcdump` snapshots by type with size bars; compare with an earlier snapshot to see growth by type.
- **Dumps**: `dotnet-dump collect` (heap, mini, triage or full); each dump is summarised with SOS `clrthreads` and `dumpheap -stat`. **Open SOS Console** opens `dotnet-dump analyze` in the terminal for `clrstack`, `dumpheap`, `eeheap` and the rest; **Download Symbols** runs `dotnet-symbol` into a folder next to the dump.
- **CPU Trace** (PRF-2, `src/features/dotnet/diagnostics/TraceView.tsx`, `crates/ide-dotnet/src/trace_source.rs`, `crates/ide-dotnet/src/helper.rs`): an EventPipe trace of 5 to 60 seconds, recorded and read by Bayonet's .NET diagnostics helper (`dotnet/Bayonet.Diagnostics`, TraceEvent; built with the installed SDK on first use) or, where it cannot be built, by `dotnet-trace collect --format Speedscope`. Merged across threads and shown as a **Flame Graph** (callers on top, coloured by your code, .NET libraries and native frames; click selects, double-click zooms), a **Call Tree** opened along the path where your own code spends its time (dotnet-trace's `CPU_TIME` samples count as their method's own time), or a sortable **Methods** list (self and total time, "only your code"). The selected method's panel shows its total and self time and what calls it and what it calls, each clickable.
  - Frames of the solution's own code are found in its C# files: the symbol index gives the files that declare a member of that name in that type, and the file's outline the declaration with the frame's namespace, nesting and parameter count. Compiler-generated names map back to what was written: async and iterator state machines (`<RunAsync>d__0.MoveNext`), lambdas (`<>c.<Main>b__0_0`), local functions, property accessors and primary constructors. Double-click, F4 or the edit icon opens the source.
  - **Show times in the editor** (on by default) puts a lens above each sampled method (`18 % · 1.24 s · self 412 ms`) and a heat bar over its lines in the change lane, warm to hot by its share of thread time. Clicking the lens shows the method in the CPU Trace tab. Helper traces carry source lines from the PDBs: then each hot line gets its own bar and its time at the end of the line (`2.39 s · self 12 ms` for a line whose time is mostly in what it calls).
  - Helper traces also list every garbage collection; the header shows how many ran, their total pause (and its share of the trace) and the longest pause.
- **Triggers**: rules such as "CPU above 80 % for 10 s, then record a CPU trace", checked against the live counters every second, with a cool-down. dotnet-monitor applies its own collection rules only in listen mode, where each app has to be started pointed at the monitor, so the IDE evaluates the rules itself; they work for any process.
- **Files** lists what was collected for the project (kept in its cache), to reopen or delete. **Tools** shows which diagnostics tools are installed, installs missing ones with `dotnet tool install --global`, and runs `dotnet-sos install` for LLDB.
