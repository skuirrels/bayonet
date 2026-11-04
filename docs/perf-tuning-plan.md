# Performance tuning: analysis and proposal

Status: proposal, 2026-11-04. Covers features for tuning the user's .NET applications. The IDE's own performance (the `perf/` harness, `perf/budgets.json`, the memory indicator in `src/features/perf`) is out of scope except where noted.

## 1. What exists today

### .NET Diagnostics tool window

`crates/ide-dotnet/src/diagnostics.rs`, `crates/ide-host/src/ops/diagnostics.rs`, `src/features/dotnet/diagnostics` (about 3,400 lines). Every feature shells out to a global `dotnet-*` tool and parses its text or JSON output.

| Tab      | What it does                                                                                                                                                 | Source                             |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------- |
| Live     | Six headline tiles with sparklines, one large line chart with crosshair, all counters in a table. 300 points (5 minutes) per series.                         | `dotnet-monitor` livemetrics, 1 s  |
| Threads  | Thread stacks grouped by identical stack, named after the innermost user frame.                                                                              | `dotnet-stack report`              |
| Memory   | GC heap by type with size bars; diff against an earlier snapshot by type.                                                                                    | `dotnet-gcdump` report             |
| Dumps    | Collect heap/mini/triage/full; summary from SOS `clrthreads` and `dumpheap -stat`; SOS console.                                                              | `dotnet-dump`, `dotnet-symbol`     |
| Trace    | 5 to 60 s CPU sample, merged across threads into an icicle flame graph (user / .NET / native colours), click to zoom, highlight search, self-time hot spots. | `dotnet-trace --format Speedscope` |
| Triggers | Threshold rules over live counters with cool-down, firing a gcdump, dump, trace or stack capture.                                                            | Evaluated in the UI                |
| Files    | Collected artefacts per project, reopen or delete.                                                                                                           | Project cache                      |
| Tools    | Installed state of each tool, one-click global install, `dotnet-sos install`.                                                                                |                                    |

### Elsewhere

- **Run with Profiler** (`crates/ide-profiler`, `src/features/profiler`): Node, Python, JVM and Linux `perf`. It has a call tree, a method list and double-click to source. It does not support .NET.
- **Coverage** (`src/features/coverage`, `src/features/dotnet/tests/coverage.ts`): gutter and reports; relevant only as a model for editor annotations.
- **Binary log viewer**: build timeline with slowest targets. A good precedent for timeline UI.

### Planned (docs/rider-clone-spec.md, section 5.7, all P2)

| ID    | Requirement                                                     | Acceptance                 | State                                                                                       |
| ----- | --------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------- |
| PRF-1 | Live counters (CPU, GC, allocations)                            | Chart updates every second | Met                                                                                         |
| PRF-2 | CPU traces with flame graph and call tree, navigating to source | Hot method opens in editor | Met (A2, below): call tree, methods, callers and callees, source navigation and editor heat |
| PRF-3 | gcdump with type and **retention** view                         | Largest types listed       | **Partly met**: types only, no retention                                                    |

The spec also names `Microsoft.Diagnostics.NETCore.Client` as a dependency and a `DotnetIDEA.Host` .NET sidecar. Neither exists yet: the repo has no `.csproj` outside `fixtures/`.

## 2. Gaps

Measured against what a developer needs to find and fix a performance problem, and against dotTrace, dotMemory and PerfView:

1. **No source link.** Speedscope frames carry no file or line, and neither the flame graph nor the hot-spot table opens code. This is the single biggest usability gap.
2. **No time dimension in traces.** A trace is aggregated into one flame graph. You cannot see _when_ something happened, correlate a CPU spike with a GC or a slow request, or select a time range.
3. **No GC analysis.** No per-GC events (generation, reason, pause, promoted bytes), no pause-time percentage, no LOH or POH breakdown, no fragmentation.
4. **No allocation profiling.** You can see what is live, but not what allocates or from where. Allocation pressure is the usual root cause of GC cost.
5. **No retention.** gcdump is a heap graph, but only the type histogram is shown. No retained size, dominator tree or path to GC root, so leaks can be seen but not explained.
6. **No wait, lock or async analysis.** Thread pool starvation, lock contention and sync-over-async are invisible to a CPU sampler.
7. **No request-level view.** ASP.NET Core, HttpClient, EF Core and SQL activity are not shown, though they are where most app latency lives.
8. **No profiling from launch.** Everything attaches to a running process, so startup cannot be profiled, and there is no "Profile" action on .NET run configurations.
9. **No comparison of traces** and no benchmark integration, so before/after work is manual.
10. **No interpretation.** The tool shows data but does not say what is wrong.
11. **Scale and polish.** Charts and the flame graph are SVG drawn by hand in two separate implementations (`dotnet/diagnostics/charts.tsx`, `profiler/ProfilerView.tsx`). SVG will not hold a full-resolution trace (tens of thousands of frames, hundreds of thousands of events); the flame graph already prunes frames below 0.2 % to cope.

## 3. Enabler: a .NET diagnostics sidecar

Most gaps cannot be closed by parsing CLI text. The MIT-licensed libraries that PerfView and dotnet-monitor are built on can:

- `Microsoft.Diagnostics.NETCore.Client`: start EventPipe sessions with any providers, start an app suspended for launch profiling, trigger gcdumps, all in-process.
- `Microsoft.Diagnostics.Tracing.TraceEvent`: parse `.nettrace` into GC, allocation, exception, contention, thread pool, `Microsoft-Diagnostics-DiagnosticSource` (ASP.NET Core, HttpClient, EF Core) and `System.Diagnostics.Metrics` events; resolve methods to source lines through portable PDBs.
- `Microsoft.Diagnostics.Runtime` (ClrMD): walk the heap of a dump or live process for retained sizes, dominators and GC root paths.

Proposal: build `DotnetIDEA.Diagnostics`, a small .NET 10 console app speaking JSON-RPC over stdio (the repo already has `ide-jsonrpc`), shipped framework-dependent next to the IDE. It produces compact, pre-aggregated, columnar results so the UI never parses raw traces. The existing CLI paths stay as a fallback when the sidecar is unavailable. This also starts the `DotnetIDEA.Host` the spec already plans.

## 4. Proposed features

Priorities: **A** builds the core tuning loop and should come first; **B** makes it excel; **C** differentiates.

### A1. Profile from the run configuration

A **Profile** button beside Run and Debug on every .NET run configuration and test, with a mode picker: _Sampling (CPU)_, _Timeline (CPU, GC, allocations, waits, requests)_, _Memory (allocations plus snapshots)_. The sidecar starts the app with a diagnostic port so startup is captured. Stop, or a fixed duration, opens the result. Extends `ide-profiler::prepare` with a .NET row.

### A2. Source-linked call tree, flame graph and editor heat

**Status: first cut done** on the existing `dotnet-trace` data, without the sidecar (see `docs/porting.md`, CPU Trace). Method-level only: Speedscope frames carry no IL offsets, so line-level self time waits for the sidecar's nettrace and PDB reading.

- Call tree (top-down), butterfly view (callers and callees of one method) and hot-spot list, with source navigation from every row and flame cell (PRF-2).
- In the editor: a heat strip in the gutter and an inline hint per method (`412 ms self · 18 %`), coloured on one scale, for the latest profile of that file. Toggle off from the editor context menu. Reuses the coverage gutter mechanism.
- Line-level self time where PDB sequence points allow it.

### A3. Timeline

The centrepiece. One canvas-rendered view with shared time axis and synced crosshair, lanes from top to bottom:

- CPU per thread (stacked area, or per-thread rows with running / waiting / blocked state colours)
- GC: one bar per collection, height by pause, colour by generation, with heap size before and after
- Allocation rate, split by small, large and pinned object heaps
- Exceptions (ticks; hover for type and message)
- Thread pool: workers, queue length, injections
- Lock contention and waits
- Requests: ASP.NET Core requests, HttpClient calls and EF Core commands as spans, nested by `Activity` parent

Brushing a range filters every other view (flame graph, call tree, hot spots, allocations) to that range. A minimap shows the whole session; mouse wheel zooms, drag pans. Markers show when a trigger fired or Hot Reload applied.

### A4. GC and memory analysis

- **GC summary**: pause time as a percentage of wall time, pause p50 / p95 / max, collections per generation, reasons (allocation, induced, low memory), mode (workstation or server, concurrent, DATAS).
- **Allocations**: by type and by allocating stack (a flame graph weighted by bytes), from `AllocationTick` sampling, with a "show allocations of this type" pivot.
- **Retention** (PRF-3): dominator tree with retained size, shortest path to GC root for any object or type, and "why is this alive?" from any row.
- **Leak finder**: take three snapshots; types that grew in both intervals are listed first, with their retaining paths.
- Generation and LOH / POH breakdown, fragmentation, strings and arrays duplicated by value.

### B1. Performance insights

Rules over the timeline and heap that produce findings, each with severity, a plain explanation, the evidence highlighted in the timeline, and the code location:

- Thread pool starvation (queue grows while threads inject slowly; blocking frames on pool threads)
- Sync-over-async (`.Result`, `.Wait()`, `GetAwaiter().GetResult()` on pool threads)
- Gen2 or LOH pressure, with the top allocating stacks
- High GC pause percentage
- Exception storms (exceptions per second above a threshold, grouped by throw site)
- Lock convoys
- EF Core N+1 (same command shape repeated within one request), slow queries
- Large `string` or `byte[]` churn, boxing in hot paths, LINQ in hot loops (from allocation stacks)
- Regex construction in hot paths

Findings appear in a ranked list at the top of every session and as editor warnings at the location.

### B2. Compare

- **Trace diff**: pick a baseline; the flame graph colours each frame by change in self time (red slower, blue faster), the call tree gains delta columns, and the summary shows the largest regressions.
- **Snapshot diff** extends today's by-type diff with retained size and new retaining paths.
- Baselines can be pinned to a Git commit, so "compare with main" works.

### B3. Live dashboard upgrade

- Meters from `System.Diagnostics.Metrics`: ASP.NET Core request rate and duration, Kestrel connections, HttpClient, EF Core, and the app's own meters.
- **Latency heatmap** for request duration histograms (time on x, latency buckets on y, colour by count), with p50 / p95 / p99 lines on top.
- Health ring per process (CPU, memory, GC pause, thread pool, error rate) at a glance; colour changes when a threshold is crossed.
- Longer history (downsampled after five minutes), pause and scrub, markers for trigger firings.
- Several processes side by side, for Aspire app hosts.

### C1. Benchmarks

BenchmarkDotNet integration: gutter run icons on `[Benchmark]` methods and classes; a results table (mean, error, allocated, Gen0 per 1k ops) with ratio bars against the baseline method; history per benchmark across runs and commits with regression highlighting; "Profile this benchmark" runs it with the EventPipe profiler and opens A2 or A3 on the result.

### C2. Sessions and reports

Each session is saved with its configuration, commit, machine and runtime version, browsable in a session list with thumbnails of its timeline. **Export report** produces a single self-contained HTML file (summary, insights, flame graph, timeline) to share in a pull request or issue.

### C3. Load and tune loop

From a request in a `.http` file: run N concurrent requests for a duration while recording a Timeline session, then show throughput, latency percentiles and the profile together. Closes the loop between the HTTP client, the profiler and the editor.

## 5. Visual design

The aim is a profiler that is pleasant to spend an afternoon in.

- **One chart kit** (`src/ui/charts` or `src/features/perf-kit`): canvas renderers for flame graph, timeline lanes, heatmap and line charts, with SVG only for axes and labels. Both existing flame graphs move onto it. Target 60 fps pan and zoom on a one-million-sample trace (add a scenario to `perf/scenarios.ts` and a budget to `perf/budgets.json`).
- **Colour with meaning only**: a fixed categorical set for frame kinds (your code, .NET, third-party, native, GC, waiting) and one sequential ramp for heat, both defined as theme tokens so they work in every theme in `docs/themes.md`, light and dark, and pass contrast checks. Red and green are never the only signal; deltas also show a sign and an arrow.
- **Motion**: zoom and range changes animate in about 150 ms with easing; nothing animates while data streams.
- **Typography**: tabular figures for every number; units always shown; durations formatted to three significant figures (`1.24 s`, `86.3 ms`).
- **Calm density**: summary figures and insights first, detail on demand; empty states explain what the view will show and offer the one action that fills it.
- **Keyboard first**: arrows walk the call tree and flame graph, Enter opens source, `[` and `]` step through time range history, `/` searches.
- **Accessibility**: every chart has a table equivalent and screen-reader summaries.

## 6. Suggested order

1. Sidecar skeleton with launch and attach sessions, nettrace parsing and source resolution (section 3).
2. A2 source-linked call tree and editor heat, on the existing trace data first, then on sidecar data.
3. Chart kit, then A3 timeline.
4. A4 GC, allocations, retention.
5. A1 Profile button on run configurations.
6. B1 insights, B2 compare, B3 live upgrade.
7. C1 to C3.

Each step should add acceptance tests on `fixtures/dotnet/` (for example a `Fixture.Api` endpoint with a deliberate N+1, an allocation hot path and a leak, so insights have something to find), and a perf scenario for any new view that renders large data.

## 7. Open questions

- Ship the sidecar framework-dependent (needs a .NET 10 runtime, which a .NET developer has) or self-contained (about 70 MB per platform)?
- Should the Profile modes reuse the generic Run with Profiler tool window, or should .NET keep its own Diagnostics window and the generic one link into it?
- Insights rules: built in only, or user-extensible (for example as plugins via `wit/`)?
