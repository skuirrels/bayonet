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

- App icons in `src-tauri/icons/` and the README wordmark are OpenIDEA's artwork. New
  artwork is needed before any release.
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
- **MSBuild navigation (SLN-5).** In project, `.props` and `.targets` files, Go to Declaration follows a version-less `PackageReference` to its `PackageVersion` in the nearest `Directory.Packages.props`, a `$(Property)` to where the file or a `Directory.Build.props/targets` above it sets it, and `Import` / `ProjectReference` paths to their files (`src/features/dotnet/msbuild/navigation.ts`). Properties set elsewhere (SDK targets, imported files) are not followed yet.
