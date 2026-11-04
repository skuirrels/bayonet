# Porting notes

DotnetIDEA is a port of OpenIDEA. This file records how the two relate, so upstream
changes can keep flowing in.

## Base

- Upstream: <https://github.com/l0ng-ai/openidea>, remote `upstream`.
- Imported from the local checkout `/Users/tyomidi/Source/openidea` at `4796265`
  (v0.1.29), with full history.
- Merge `upstream/main` regularly (spec risk K6).

## What was renamed

User-visible names only:

- `src-tauri/tauri.conf.json`: product name, window title, bundle identifier
  (`io.github.skuirrels.dotnetidea`), main binary (`dotnetidea`), descriptions.
- Menus (`src-tauri/src/menu.rs`), locale strings (`src/platform/locales/`), title bar,
  welcome screen, memory indicator, settings import/export dialogs, splash screen.
- `README.md`, `NOTICE`.

A new bundle identifier means DotnetIDEA keeps its own settings and caches, separate
from an installed OpenIDEA.

## What deliberately kept the OpenIDEA name

Renaming these would conflict with nearly every upstream merge, and none is visible to users:

- Rust crate and module names, including the `openidea` package in `src-tauri`.
- `OPENIDEA_*` environment variables, `openidea-*` temporary file prefixes, thread names.
- The per-project folder `.openidea/` (`crates/ide-core/src/project_dir.rs` and about 80
  other references). Projects can be opened by both IDEs. Spec section 4.2 planned a
  rename to `.dotnetidea/`; that is deferred until the port has diverged enough to
  justify the merge cost.
- The plugin ABI package `openidea:plugin@0.1.0` (`wit/plugin.wit`) and protocol
  extensions such as `_openidea/steer`.
- `openidea-server`, the headless backend binary used for remote development.
- CSS class names and SVG source comments.

## Disabled

- **Auto-update.** OpenIDEA's updater feed and public key were removed from
  `tauri.conf.json`. With no endpoints, "Check for Updates" is hidden
  (`src-tauri/src/commands/window.rs`). Re-enable once DotnetIDEA has its own signed
  release feed.

## Still to do

- The app icon and splash mark are DotnetIDEA's own (".d" on a violet frame, source in
  `src-tauri/icons/source/dotnetidea.svg`; regenerate with `pnpm tauri icon` on that file).
  The `openidea-wordmark*.svg` files still spell "OpenIDEA" and need redrawing.
- `.github/workflows/release.yml` still assumes OpenIDEA's signing secrets and updater
  artifacts.

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

## .NET inner loop (phase 2)

- **Build.** `BuildSystem::Dotnet` in `crates/ide-build/src/compile/` builds the root
  solution (`.slnx` first) or, for Build Module, the file's project, with `dotnet build`
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

## Unit tests (phase 3)

- The Unit Tests tool window (bottom stripe) lists every test of the solution's test projects by project, namespace, class and method. Data-driven rows are grouped under their method. It can build and discover, run all, run the selection, stop, and filter by name or to failed tests. The selected test shows its message, expected and actual values, a stack trace whose frames open the source, and its output.
- Tests are driven through Microsoft.Testing.Platform's JSON-RPC server mode (`crates/ide-dotnet/src/testing.rs`). The host listens on a local port and starts `dotnet exec <TargetPath> --server --client-host 127.0.0.1 --client-port N`. It sends `initialize`, then `testing/discoverTests` or `testing/runTests` (with the selected uids), and folds the `testing/testUpdates/tests` notifications into the tree. This gives each test's type, method, file and line, which `dotnet test`'s console output does not. NUnit reports no file, so its locations are found in the project's sources.
- C# (and VB, F#) editors get gutter run icons on test methods and classes from the discovered locations, coloured by the last result. Clicking one runs just those tests.
- Not yet: VSTest-only test projects (they show "only Microsoft.Testing.Platform test projects are supported so far"), debugging a test (needs netcoredbg, DBG-5), and continuous testing (TST-3).

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
- **Source generators (CI-17).** A C# or VB project in the Solution Explorer starts with **Dependencies › Analyzers**: one folder per analyser assembly, and below a generator its files (`<assembly>/<generator type>/<hint name>`), read-only. Roslyn keeps generated sources in memory, so the IDE compiles the project once more with `EmitCompilerGeneratedFiles` into its build cache (`crates/ide-dotnet/src/generators.rs`, `src/features/dotnet/generated.ts`). An import passed as `CustomAfterMicrosoftCommonTargets` and conditioned on the project's path moves only that project's intermediate folder there, so its `obj` and `bin` (and those of the projects it references) are not touched. This runs when the node is first expanded, and again shortly after the project's sources change while it is expanded. Go to Declaration into generated code opens Roslyn's own copy (`roslyn-source-generated://`, read with `sourceGeneratedDocument/_roslyn_getText` and never cached), with a banner naming the generator. Packages, project references and frameworks are not listed under Dependencies yet.
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
- Not verified end to end: netcoredbg is not installed on the development machine.

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
- **CPU Trace**: `dotnet-trace collect --format Speedscope` for 5 to 60 seconds, merged across threads into a flame graph (callers on top; click to zoom), coloured by your code, .NET libraries and native frames, with a self-time hot-spot table.
- **Triggers**: rules such as "CPU above 80 % for 10 s, then record a CPU trace", checked against the live counters every second, with a cool-down. dotnet-monitor applies its own collection rules only in listen mode, where each app has to be started pointed at the monitor, so the IDE evaluates the rules itself; they work for any process.
- **Files** lists what was collected for the project (kept in its cache), to reopen or delete. **Tools** shows which diagnostics tools are installed, installs missing ones with `dotnet tool install --global`, and runs `dotnet-sos install` for LLDB.
