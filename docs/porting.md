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
