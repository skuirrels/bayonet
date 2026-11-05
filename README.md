# Bayonet

An open-source .NET IDE with a Rust core, built on Tauri.

Bayonet is a port of [OpenIDEA](https://github.com/l0ng-ai/openidea), refocused on
.NET: C#, F#, solutions and projects, NuGet, unit tests and debugging. It keeps OpenIDEA's
editor, tool windows, Git, terminal, database and HTTP clients.

Status: early. See [the specification](docs/rider-clone-spec.md) for the plan and
[porting notes](docs/porting.md) for how the code relates to OpenIDEA.

## Build

```sh
pnpm install
pnpm tauri dev
```

Rust 1.96 or later, Node 22 or later, pnpm, the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and, for .NET work,
the .NET 10 SDK.

Native notebook execution tests are opt-in and need two distinct Python environments
with `ipykernel` installed:

```sh
OPENIDEA_TEST_PYTHON=/first/env/bin/python \
OPENIDEA_TEST_PYTHON_SECOND=/second/env/bin/python \
cargo test -p ide-notebook --test kernel_runtime -- --ignored --test-threads=1
```

## Plugins

Plugins add actions, completion, quick fixes, tool windows, custom editors,
themes, file icons and languages. They run as sandboxed WebAssembly components;
themes, icon themes and languages need no code at all.

Install them from Settings › Plugins, or start your own with
`openidea-plugin new` — see [Writing OpenIDEA plugins](plugins/README.md) and
the [official plugins](plugins/official). The plugin API is still a preview.

## License

[Apache 2.0](LICENSE). See [`NOTICE`](NOTICE) for attribution, including OpenIDEA, and
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) for bundled third-party assets.

<sub>Not affiliated with JetBrains or Microsoft. IntelliJ IDEA and Rider are trademarks of JetBrains s.r.o.</sub>
