//! Drives real debug adapters against tiny programs (one per language), checking the
//! launch templates and the IDEA-parity features: line / method breakpoints, Smart Step
//! Into, Reset Frame, Force Step Over, Force Run to Cursor and Set Value.
//!
//! Ignored by default (needs the adapters installed). Run with
//! `OPENIDEA_DAP_FIXTURES=<dir> cargo test -p ide-dap --test real_adapters -- --ignored --nocapture`
//! where `<dir>` holds `py/app.py`, `node/app.js`, `go/demo` (built with `-N -l`),
//! `rust/target/debug/demo` and `c/main`. Adapters not found on `PATH` are skipped.

use ide_core::event::DebugEvent;
use ide_core::{DebugSessionId, IdeEvent, SharedSink};
use ide_dap::*;
use ide_language::LanguageRegistry;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Case {
    adapter: &'static str,
    /// Source file and the 0-based line of `total += parse(...)` inside the loop.
    source: PathBuf,
    loop_line: u32,
    /// Line of the `return total` after the loop (Force Run to Cursor target).
    after_loop: u32,
    function: &'static str,
    /// Persistent field watchpoint id, when the adapter has one (java-debug).
    field: Option<&'static str>,
    config: LaunchConfig,
}

fn launch(adapter: &str, program: &Path, cwd: &Path, extra: serde_json::Value) -> LaunchConfig {
    LaunchConfig {
        name: format!("{adapter} fixture"),
        adapter: adapter.into(),
        request: "launch".into(),
        program: Some(program.display().to_string()),
        args: vec![],
        cwd: Some(cwd.into()),
        env: BTreeMap::from([("OPENIDEA_FIXTURE".into(), "1".into())]),
        extra_json: Some(extra.to_string()),
    }
}

fn cases(dir: &Path) -> Vec<Case> {
    let py = dir.join("py");
    let node = dir.join("node");
    let go = dir.join("go");
    let rust = dir.join("rust");
    let c = dir.join("c");
    let java = dir.join("java");
    // java-debug runs inside jdtls: OPENIDEA_JAVA_DEBUG_SERVER is the port the IDE's
    // `java_debug_server` returned for this folder (classes compiled to java/out).
    let java_case =
        std::env::var("OPENIDEA_JAVA_DEBUG_SERVER").ok().and_then(|p| p.parse::<u16>().ok()).map(|port| Case {
            adapter: "java-debug",
            source: java.join("src/demo/Main.java"),
            loop_line: 14,
            after_loop: 16,
            function: "demo.Main#parse",
            field: Some("demo.Main#count"),
            config: LaunchConfig {
                program: None,
                ..launch(
                    "java-debug",
                    Path::new(""),
                    &java,
                    serde_json::json!({
                        "debugServer": port,
                        "mainClass": "demo.Main",
                        "classPaths": [java.join("out")],
                        "console": "internalConsole",
                    }),
                )
            },
        });
    let mut out: Vec<Case> = java_case.into_iter().collect();
    out.extend(vec![
        Case {
            adapter: "debugpy",
            source: py.join("app.py"),
            loop_line: 8,
            after_loop: 9,
            function: "parse",
            field: None,
            config: launch(
                "debugpy",
                &py.join("app.py"),
                &py,
                serde_json::json!({
                    "console": "internalConsole",
                    "justMyCode": true,
                    "redirectOutput": true,
                    "python": python(),
                }),
            ),
        },
        Case {
            adapter: "js-debug",
            source: node.join("app.js"),
            loop_line: 8,
            after_loop: 10,
            function: "parse",
            field: None,
            config: launch(
                "js-debug",
                &node.join("app.js"),
                &node,
                serde_json::json!({"type": "pwa-node", "console": "internalConsole", "outputCapture": "std"}),
            ),
        },
        Case {
            adapter: "delve",
            source: go.join("main.go"),
            loop_line: 15,
            after_loop: 17,
            function: "main.parse",
            field: None,
            config: launch("delve", &go.join("demo"), &go, serde_json::json!({"mode": "exec"})),
        },
        Case {
            adapter: "codelldb",
            source: rust.join("src/main.rs"),
            loop_line: 8,
            after_loop: 10,
            function: "parse",
            field: None,
            config: launch(
                "codelldb",
                &rust.join("target/debug/demo"),
                &rust,
                serde_json::json!({"sourceLanguages": ["rust"]}),
            ),
        },
        Case {
            adapter: "codelldb",
            source: c.join("main.c"),
            loop_line: 13,
            after_loop: 15,
            function: "parse",
            field: None,
            config: launch("codelldb", &c.join("main"), &c, serde_json::json!({})),
        },
        Case {
            adapter: "lldb-dap",
            source: c.join("main.c"),
            loop_line: 13,
            after_loop: 15,
            function: "parse",
            field: None,
            config: launch("lldb-dap", &c.join("main"), &c, serde_json::json!({"env": ["OPENIDEA_FIXTURE=1"]})),
        },
    ]);
    out
}

struct Run {
    manager: Arc<DebugManager>,
    events: Arc<Mutex<Vec<(DebugSessionId, DebugEvent)>>>,
    owner: SessionOwner,
}

impl Run {
    fn new() -> Self {
        let events: Arc<Mutex<Vec<(DebugSessionId, DebugEvent)>>> = Arc::default();
        let sink_events = events.clone();
        let sink: SharedSink = Arc::new(move |e: IdeEvent| {
            if let IdeEvent::Debug { session, event } = e {
                sink_events.lock().push((session, event));
            }
        });
        let languages = Arc::new(LanguageRegistry::from_configs(Vec::new()));
        Self { manager: DebugManager::new(languages), events, owner: SessionOwner::new("test", sink) }
    }

    /// Waits for a `stopped` event after `seen` events; returns (session, thread, reason).
    async fn stopped_after(&self, seen: usize) -> Option<(DebugSessionId, i64, String)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let found = self.events.lock().iter().skip(seen).find_map(|(s, e)| match e {
                DebugEvent::Stopped { thread_id, reason, .. } => Some((*s, *thread_id, reason.clone())),
                _ => None,
            });
            if found.is_some() {
                return found;
            }
            if self.events.lock().iter().skip(seen).any(|(_, e)| matches!(e, DebugEvent::Terminated)) {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }

    fn seen(&self) -> usize {
        self.events.lock().len()
    }

    async fn top(&self, s: DebugSessionId, thread: i64) -> StackFrame {
        self.manager.stack_trace(s, thread).await.unwrap().into_iter().next().unwrap()
    }
}

/// Bin dirs of the IDE's managed tools (the app prepends them to `PATH` at startup).
fn add_managed_tools_to_path() {
    // `RUST_LOG=ide_dap::wire=trace` prints the DAP traffic.
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).try_init();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    // Like the app: adapters installed by OpenIDEA are found without being on PATH.
    adapter::set_tools_root(home.join("Library/Application Support/ai.l0ng.openidea/tools"));
}

/// The debuggee interpreter (`OPENIDEA_PYTHON`, e.g. one without debugpy).
fn python() -> String {
    std::env::var("OPENIDEA_PYTHON").unwrap_or_else(|_| "python3".into())
}

fn installed(adapter: &str) -> bool {
    let cmd = match adapter {
        "debugpy" => {
            let managed = adapter::tools_root()
                .is_some_and(|r| r.join(adapter::DEBUGPY_TOOL).join("lib/debugpy/__init__.py").is_file());
            return managed
                || std::process::Command::new(python())
                    .args(["-c", "import debugpy"])
                    .status()
                    .is_ok_and(|s| s.success());
        }
        "js-debug" => "js-debug-adapter",
        "delve" => "dlv",
        "java-debug" => return true,
        other => other,
    };
    adapter::resolve_binary(cmd, Path::new("/")).is_some()
}

async fn scenario(case: &Case) -> Result<Vec<String>, String> {
    let mut report = Vec::new();
    let run = Run::new();
    let m = &run.manager;
    let line_bp =
        Breakpoint { line: case.loop_line, condition: None, hit_condition: None, log_message: None, enabled: true };
    m.set_breakpoints(case.source.clone(), vec![line_bp]).await.map_err(|e| e.message)?;

    let seen = run.seen();
    let id = m.start(case.config.clone(), run.owner.clone()).await.map_err(|e| format!("start: {}", e.message))?;
    let Some((s, thread, reason)) = run.stopped_after(seen).await else {
        let log: Vec<String> = run.events.lock().iter().map(|(_, e)| format!("{e:?}")).collect();
        return Err(format!(
            "no stop at the line breakpoint; events: {log:#?}; states: {:?}",
            m.breakpoint_states(&case.source)
        ));
    };
    let top = run.top(s, thread).await;
    if top.line != case.loop_line {
        return Err(format!("stopped at line {} ({reason}), expected {}", top.line, case.loop_line));
    }
    report.push(format!("launch + line breakpoint: stopped at {}:{}", top.name, top.line + 1));
    let caps = m.capabilities(s).map_err(|e| e.message)?;
    report.push(format!("capabilities: {caps:?}"));

    // Set Value on the first local that is an integer.
    let scopes = m.scopes(s, top.id).await.map_err(|e| e.message)?;
    for scope in scopes.iter().filter(|sc| !sc.expensive) {
        let vars = m.variables(s, scope.variables_reference).await.map_err(|e| e.message)?;
        if let Some(v) = vars.iter().find(|v| v.name == "total") {
            match m
                .set_variable(
                    s,
                    scope.variables_reference,
                    v.name.clone(),
                    v.evaluate_name.clone(),
                    Some(top.id),
                    "40".into(),
                )
                .await
            {
                Ok(nv) => report.push(format!("set value: total = {}", nv.value)),
                Err(e) => report.push(format!("set value: FAILED {}", e.message)),
            }
            break;
        }
    }

    // Reset Frame: one line further, then back to the start of `run`.
    if caps.restart_frame {
        let seen = run.seen();
        m.step(s, thread, StepKind::Over).await.map_err(|e| e.message)?;
        run.stopped_after(seen).await.ok_or("no stop after step over")?;
        let before = run.top(s, thread).await;
        let seen = run.seen();
        match m.restart_frame(s, before.id).await {
            Ok(()) => {
                run.stopped_after(seen).await.ok_or("no stop after reset frame")?;
                let r = run.top(s, thread).await;
                report.push(format!("reset frame: {}:{} -> {}:{}", before.name, before.line + 1, r.name, r.line + 1));
            }
            Err(e) => report.push(format!("reset frame: FAILED {}", e.message)),
        }
        let seen = run.seen();
        m.resume(s, thread).await.map_err(|e| e.message)?;
        run.stopped_after(seen).await.ok_or("no stop at the loop after reset frame")?;
    }

    // Smart Step Into.
    if caps.step_in_targets {
        let targets = m.step_in_targets(s, top.id).await.map_err(|e| e.message)?;
        report.push(format!("step-in targets: {:?}", targets.iter().map(|t| t.label.as_str()).collect::<Vec<_>>()));
        if let Some(t) = targets.iter().find(|t| t.label.contains("parse")) {
            let seen = run.seen();
            m.step_in_target(s, thread, t.id).await.map_err(|e| e.message)?;
            run.stopped_after(seen).await.ok_or("no stop after smart step into")?;
            let f = run.top(s, thread).await;
            report.push(format!("smart step into: now in {}", f.name));
            // Reset Frame back to the start of parse.
            if caps.restart_frame {
                let seen = run.seen();
                match m.restart_frame(s, f.id).await {
                    Ok(()) => {
                        run.stopped_after(seen).await.ok_or("no stop after reset frame")?;
                        let r = run.top(s, thread).await;
                        report.push(format!("reset frame: at {}:{}", r.name, r.line + 1));
                    }
                    Err(e) => report.push(format!("reset frame: FAILED {}", e.message)),
                }
            }
            let seen = run.seen();
            m.step(s, thread, StepKind::Out).await.map_err(|e| e.message)?;
            run.stopped_after(seen).await.ok_or("no stop after step out")?;
        }
    } else {
        report.push("step-in targets: not supported".into());
    }

    // Method breakpoint: continue → stops in `parse`.
    if caps.function_breakpoints {
        let fb = FunctionBreakpoint { name: case.function.into(), condition: None, hit_condition: None, enabled: true };
        let states = m.set_function_breakpoints(vec![fb]).await.map_err(|e| e.message)?;
        report.push(format!("method breakpoint `{}`: verified={}", case.function, states[0].verified));
        m.set_breakpoints(case.source.clone(), vec![]).await.map_err(|e| e.message)?;
        let seen = run.seen();
        m.resume(s, thread).await.map_err(|e| e.message)?;
        match run.stopped_after(seen).await {
            Some((_, t, reason)) => {
                let f = run.top(s, t).await;
                report.push(format!("method breakpoint hit: {reason} in {}", f.name));
            }
            None => report.push("method breakpoint: NOT HIT".into()),
        }
        m.set_function_breakpoints(vec![]).await.map_err(|e| e.message)?;
        m.set_breakpoints(case.source.clone(), vec![line_bp_at(case.loop_line)]).await.map_err(|e| e.message)?;
        let seen = run.seen();
        m.step(s, thread, StepKind::Out).await.map_err(|e| e.message)?;
        run.stopped_after(seen).await.ok_or("no stop after step out of parse")?;
    } else {
        report.push("method breakpoints: not supported".into());
    }

    // Force Step Over ignores the breakpoint inside parse.
    m.set_breakpoints(case.source.clone(), vec![line_bp_at(case.loop_line)]).await.map_err(|e| e.message)?;
    let seen = run.seen();
    m.force_step_over(s, thread).await.map_err(|e| e.message)?;
    let (_, t, reason) = run.stopped_after(seen).await.ok_or("no stop after force step over")?;
    let f = run.top(s, t).await;
    report.push(format!("force step over: {reason} at line {}", f.line + 1));

    // Field watchpoint / data breakpoint on `total` (java-debug: a field id).
    if caps.data_breakpoints {
        let frame = run.top(s, thread).await;
        let mut data_id = case.field.map(str::to_owned);
        if data_id.is_none() {
            for scope in m.scopes(s, frame.id).await.map_err(|e| e.message)?.iter().filter(|sc| !sc.expensive) {
                if let Ok(info) =
                    m.data_breakpoint_info(s, scope.variables_reference, "total".into(), Some(frame.id)).await
                    && info.data_id.is_some()
                {
                    data_id = info.data_id;
                    break;
                }
            }
        }
        match data_id {
            Some(data_id) => {
                let wp = DataBreakpoint {
                    data_id: data_id.clone(),
                    access_type: Some("write".into()),
                    condition: None,
                    hit_condition: None,
                    enabled: true,
                };
                let states = m.set_data_breakpoints(vec![wp]).await.map_err(|e| e.message)?;
                m.set_breakpoints(case.source.clone(), vec![]).await.map_err(|e| e.message)?;
                let seen = run.seen();
                m.resume(s, thread).await.map_err(|e| e.message)?;
                match run.stopped_after(seen).await {
                    Some((_, t, reason)) => {
                        let f = run.top(s, t).await;
                        report.push(format!(
                            "watchpoint `{data_id}` (verified={}): {reason} at {}:{}",
                            states[0].verified,
                            f.name,
                            f.line + 1
                        ));
                    }
                    None => report.push(format!("watchpoint `{data_id}`: NOT HIT")),
                }
                m.set_data_breakpoints(vec![]).await.map_err(|e| e.message)?;
                m.set_breakpoints(case.source.clone(), vec![line_bp_at(case.loop_line)])
                    .await
                    .map_err(|e| e.message)?;
            }
            None => report.push("watchpoint: no data id for `total`".into()),
        }
    }

    // Force Run to Cursor past the loop, ignoring the loop breakpoint.
    let seen = run.seen();
    m.run_to(s, thread, case.source.clone(), case.after_loop, true).await.map_err(|e| e.message)?;
    let (_, t, reason) = run.stopped_after(seen).await.ok_or("no stop after force run to cursor")?;
    let f = run.top(s, t).await;
    report.push(format!("force run to cursor: {reason} at line {} (want {})", f.line + 1, case.after_loop + 1));

    m.stop(id).await.map_err(|e| e.message)?;
    Ok(report)
}

fn line_bp_at(line: u32) -> Breakpoint {
    Breakpoint { line, condition: None, hit_condition: None, log_message: None, enabled: true }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs real debug adapters and OPENIDEA_DAP_FIXTURES"]
async fn real_adapters() {
    let Some(dir) = std::env::var_os("OPENIDEA_DAP_FIXTURES").map(PathBuf::from) else {
        eprintln!("OPENIDEA_DAP_FIXTURES not set");
        return;
    };
    add_managed_tools_to_path();
    let only = std::env::var("OPENIDEA_DAP_ONLY").ok();
    let mut failures = Vec::new();
    for case in cases(&dir) {
        if only.as_deref().is_some_and(|o| o != case.adapter) {
            continue;
        }
        let name = format!("{} ({})", case.adapter, case.source.file_name().unwrap().to_string_lossy());
        if !installed(case.adapter) {
            println!("== {name}: SKIPPED (not installed)");
            continue;
        }
        match tokio::time::timeout(Duration::from_secs(240), scenario(&case)).await {
            Ok(Ok(lines)) => {
                println!("== {name}: OK");
                for l in lines {
                    println!("   {l}");
                }
            }
            Ok(Err(e)) => {
                println!("== {name}: FAILED {e}");
                failures.push(name);
            }
            Err(_) => {
                println!("== {name}: TIMED OUT");
                failures.push(name);
            }
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}

struct AttachCase {
    kind: attach::AttachKind,
    command: Vec<String>,
    source: PathBuf,
    /// 0-based line inside `tick`.
    line: u32,
    extra: fn(u32, Option<u16>) -> serde_json::Value,
}

/// Attach to Process: each fixture server is listed with the right debugger, and attaching
/// with the request `attach.ts` builds stops at a breakpoint in its loop.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs real debug adapters and OPENIDEA_DAP_FIXTURES"]
async fn attach_to_processes() {
    let Some(dir) = std::env::var_os("OPENIDEA_DAP_FIXTURES").map(PathBuf::from) else { return };
    add_managed_tools_to_path();
    let s = |p: &Path| p.display().to_string();
    let java_server = std::env::var("OPENIDEA_JAVA_DEBUG_SERVER").ok().and_then(|p| p.parse::<u16>().ok());
    let mut cases = vec![
        // `debugpy --pid` injection ships only an x86_64 helper on macOS, so on Apple
        // Silicon the attach goes through a `debugpy --listen` process.
        AttachCase {
            kind: attach::AttachKind::Python,
            command: vec![
                "python3".into(),
                "-m".into(),
                "debugpy".into(),
                "--listen".into(),
                "127.0.0.1:5931".into(),
                s(&dir.join("py/server.py")),
            ],
            source: dir.join("py/server.py"),
            line: 4,
            extra: |pid, port| match port {
                // `debugpy --listen` runs its own adapter on that port: talk DAP to it directly.
                Some(port) => serde_json::json!({"debugServer": port, "justMyCode": true}),
                None => serde_json::json!({"processId": pid, "justMyCode": true}),
            },
        },
        AttachCase {
            kind: attach::AttachKind::Node,
            command: match std::env::var("OPENIDEA_NODE_INSPECT") {
                Ok(flag) => vec!["node".into(), flag, s(&dir.join("node/server.js"))],
                Err(_) => vec!["node".into(), s(&dir.join("node/server.js"))],
            },
            source: dir.join("node/server.js"),
            line: 2,
            extra: |pid, port| match port {
                Some(port) => {
                    serde_json::json!({"type": "pwa-node", "address": "127.0.0.1", "port": port, "continueOnAttach": true})
                }
                None => serde_json::json!({"type": "pwa-node", "processId": pid.to_string(), "continueOnAttach": true}),
            },
        },
        AttachCase {
            kind: attach::AttachKind::Go,
            command: vec![s(&dir.join("goserver/server"))],
            source: dir.join("goserver/main.go"),
            line: 5,
            extra: |pid, _| serde_json::json!({"mode": "local", "processId": pid}),
        },
        AttachCase {
            kind: attach::AttachKind::Native,
            command: vec![s(&dir.join("c/server"))],
            source: dir.join("c/server.c"),
            line: 3,
            extra: |pid, _| serde_json::json!({"pid": pid}),
        },
    ];
    if java_server.is_some() {
        cases.insert(
            0,
            AttachCase {
                kind: attach::AttachKind::Java,
                command: vec![
                    "java".into(),
                    "-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:5917".into(),
                    "-cp".into(),
                    s(&dir.join("java/out")),
                    "demo.Server".into(),
                ],
                source: dir.join("java/src/demo/Server.java"),
                line: 4,
                extra: |_, port| serde_json::json!({"hostName": "localhost", "port": port}),
            },
        );
    }
    let only = std::env::var("OPENIDEA_DAP_ONLY").ok();
    let mut failures = Vec::new();
    for case in cases {
        let label = format!("{:?}", case.kind);
        if only.as_deref().is_some_and(|o| !label.eq_ignore_ascii_case(o)) {
            continue;
        }
        let mut child = std::process::Command::new(&case.command[0])
            .args(&case.command[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let result = async {
            let list =
                tokio::task::spawn_blocking(attach::attachable_processes).await.unwrap().map_err(|e| e.message)?;
            let p = list.iter().find(|p| p.pid == child.id()).ok_or("process not listed")?;
            if p.kind != case.kind {
                return Err(format!("listed as {:?}", p.kind));
            }
            let mut extra = (case.extra)(p.pid, p.port);
            if p.kind == attach::AttachKind::Java {
                extra["debugServer"] = java_server.unwrap().into();
            }
            let config = LaunchConfig {
                name: format!("{} ({})", p.name, p.pid),
                adapter: p.adapter.clone(),
                request: "attach".into(),
                program: None,
                args: vec![],
                cwd: case.source.parent().map(Into::into),
                env: BTreeMap::new(),
                extra_json: Some(extra.to_string()),
            };
            let run = Run::new();
            run.manager
                .set_breakpoints(case.source.clone(), vec![line_bp_at(case.line)])
                .await
                .map_err(|e| e.message)?;
            let id =
                run.manager.start(config, run.owner.clone()).await.map_err(|e| format!("attach: {}", e.message))?;
            let Some((s, thread, reason)) = run.stopped_after(0).await else {
                let mut log: Vec<String> = run.events.lock().iter().map(|(_, e)| format!("{e:?}")).collect();
                for info in run.manager.sessions(None) {
                    let threads = run.manager.threads(info.id).await;
                    log.push(format!("session {:?} threads {threads:?}", info.id));
                    if let Ok(t) = threads
                        && let Some(t) = t.first()
                    {
                        let seen = run.seen();
                        let _ = run.manager.pause(info.id, t.id).await;
                        if let Some((s, th, r)) = run.stopped_after(seen).await {
                            let frames = run.manager.stack_trace(s, th).await;
                            log.push(format!(
                                "paused ({r}): {:?}",
                                frames.map(|f| f
                                    .into_iter()
                                    .take(3)
                                    .map(|f| (f.name, f.path, f.line))
                                    .collect::<Vec<_>>())
                            ));
                        }
                    }
                }
                log.push(format!("states: {:?}", run.manager.breakpoint_states(&case.source)));
                let _ = run.manager.stop(id).await;
                return Err(format!("no stop after attaching: {log:#?}"));
            };
            let top = run.top(s, thread).await;
            let _ = run.manager.stop(id).await;
            Ok::<_, String>(format!("{} via {}: {reason} at {}:{}", p.name, p.adapter, top.name, top.line + 1))
        };
        let outcome = tokio::time::timeout(Duration::from_secs(150), result).await;
        let _ = child.kill();
        let _ = child.wait();
        match outcome {
            Ok(Ok(line)) => println!("== attach {label}: OK {line}"),
            Ok(Err(e)) => {
                println!("== attach {label}: FAILED {e}");
                failures.push(label);
            }
            Err(_) => {
                println!("== attach {label}: TIMED OUT");
                failures.push(label);
            }
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}

/// Parallel Stacks (DBG-6) needs every thread's stack while stopped: four workers parked in
/// `wait_here` (through `worker`) and `main` in `pthread_join` all report their own frames.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs lldb-dap and clang"]
async fn lldb_dap_reports_every_threads_stack() {
    add_managed_tools_to_path();
    if !installed("lldb-dap") {
        eprintln!("lldb-dap not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("openidea-parallel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("threads.c");
    std::fs::write(
        &source,
        r#"#include <pthread.h>
#include <stdatomic.h>
#include <unistd.h>

static atomic_int started;
static volatile int spin = 1;

static void wait_here(int id) {
    while (spin) usleep(1000 * (id + 1));
}

static void *worker(void *arg) {
    atomic_fetch_add(&started, 1);
    wait_here((int)(long)arg);
    return 0;
}

int main(void) {
    pthread_t t[4];
    for (long i = 0; i < 4; i++) pthread_create(&t[i], 0, worker, (void *)i);
    while (atomic_load(&started) < 4) usleep(1000);
    usleep(100000);
    int stop = 0; // break here
    for (int i = 0; i < 4; i++) pthread_join(t[i], 0);
    return stop;
}
"#,
    )
    .unwrap();
    let program = dir.join("threads");
    let built = std::process::Command::new("clang")
        .args(["-g", "-O0", "-pthread", "-o"])
        .arg(&program)
        .arg(&source)
        .status()
        .is_ok_and(|s| s.success());
    assert!(built, "clang could not build the fixture");
    let run = Run::new();
    let m = &run.manager;
    // 0-based line of `int stop = 0;`.
    m.set_breakpoints(source.clone(), vec![line_bp_at(22)]).await.unwrap();
    let seen = run.seen();
    m.start(launch("lldb-dap", &program, &dir, serde_json::json!({})), run.owner.clone()).await.unwrap();
    let (s, thread, _) = run.stopped_after(seen).await.expect("stopped at the breakpoint");
    assert_eq!(run.top(s, thread).await.line, 22);
    let threads = m.threads(s).await.unwrap();
    let mut in_worker = 0;
    for t in &threads {
        let frames = m.stack_trace(s, t.id).await.unwrap();
        let names: Vec<&str> = frames.iter().map(|f| f.name.as_str()).collect();
        if names.iter().any(|n| n.contains("wait_here")) {
            assert!(names.iter().any(|n| n.contains("worker")), "{names:?}");
            in_worker += 1;
        }
    }
    assert_eq!(in_worker, 4, "{} threads", threads.len());
    m.stop(s).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

// --- netcoredbg ------------------------------------------------------------------------

const DEMO_CSPROJ: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net10.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>
</Project>
"#;

const DEMO_PROGRAM: &str = r#"record Order(int Id, decimal Total);

static class Program
{
    static readonly CountdownEvent Started = new(4);
    static readonly ManualResetEventSlim Gate = new(false);

    static void Park(int id) => Gate.Wait();

    static void Worker(object? id)
    {
        Started.Signal();
        Park((int)id!);
    }

    static decimal Tick(decimal t)
    {
        Thread.Sleep(50);
        return t + 1; // tick
    }

    static int Main(string[] args)
    {
        var total = 0m;
        if (args.Length > 0 && args[0] == "wait")
            while (total < 100_000) total = Tick(total);
        var workers = new Thread[4];
        for (var i = 0; i < 4; i++)
        {
            workers[i] = new Thread(Worker) { Name = $"Worker {i}" };
            workers[i].Start(i);
        }
        Started.Wait();
        var orders = new List<Order> { new(1, 12.5m), new(2, 3m), new(3, 40m) };
        total = orders.Sum(o => o.Total); // break here
        total += 1; // next
        Gate.Set();
        foreach (var w in workers) w.Join();
        return total > 0 ? 0 : 1;
    }
}
"#;

/// 0-based line of the line containing `marker`.
fn line_of(text: &str, marker: &str) -> u32 {
    text.lines().position(|l| l.contains(marker)).unwrap_or_else(|| panic!("no {marker}")) as u32
}

/// A console project in a fresh temp directory (`Demo.csproj`, `Program.cs`).
fn demo_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openidea-netcoredbg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Demo.csproj"), DEMO_CSPROJ).unwrap();
    std::fs::write(dir.join("Program.cs"), DEMO_PROGRAM).unwrap();
    real(dir)
}

/// The compiler records real paths and netcoredbg matches breakpoints by path, so the
/// temp directory's `/var` → `/private/var` symlink must be resolved.
fn real(dir: PathBuf) -> PathBuf {
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

fn netcoredbg(request: &str, cwd: &Path, extra: serde_json::Value) -> LaunchConfig {
    LaunchConfig {
        name: "netcoredbg".into(),
        adapter: "netcoredbg".into(),
        request: request.into(),
        program: None,
        args: vec![],
        cwd: Some(cwd.into()),
        env: BTreeMap::new(),
        extra_json: Some(extra.to_string()),
    }
}

/// Waits up to `secs` for a `stopped` event after `seen`.
async fn stopped_within(run: &Run, seen: usize, secs: u64) -> Option<(DebugSessionId, i64, String)> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(found) = run.stopped_after(seen).await {
            return Some(found);
        }
    }
    None
}

/// DBG-1/3/6 with netcoredbg: a `dotnet` launch builds and runs the project and stops at a
/// breakpoint; every worker thread's stack is readable (Parallel Stacks); a `List<T>`'s
/// children are named `[0]`… with their own members (the collection visualiser's input);
/// Set Next Statement works when netcoredbg advertises `gotoTargets`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs netcoredbg and the .NET SDK"]
async fn netcoredbg_launches_a_project_and_reads_threads_and_collections() {
    add_managed_tools_to_path();
    if !installed("netcoredbg") {
        eprintln!("netcoredbg not installed");
        return;
    }
    let dir = demo_project("launch");
    let source = dir.join("Program.cs");
    let at = line_of(DEMO_PROGRAM, "// break here");
    let run = Run::new();
    let m = &run.manager;
    m.set_breakpoints(source.clone(), vec![line_bp_at(at)]).await.unwrap();
    let seen = run.seen();
    let config = netcoredbg("launch", &dir, serde_json::json!({ "dotnet": { "project": dir } }));
    m.start(config, run.owner.clone()).await.expect("start");
    let (s, thread, reason) = stopped_within(&run, seen, 120).await.expect("stopped at the breakpoint");
    println!("stopped: {reason}");
    let top = run.top(s, thread).await;
    assert_eq!(top.line, at, "{top:?}");

    let threads = m.threads(s).await.unwrap();
    let mut parked = 0;
    for t in &threads {
        let names: Vec<String> = m.stack_trace(s, t.id).await.unwrap().into_iter().map(|f| f.name).collect();
        if names.iter().any(|n| n.contains("Program.Park")) {
            assert!(names.iter().any(|n| n.contains("Program.Worker")), "{names:?}");
            parked += 1;
        }
    }
    assert_eq!(parked, 4, "{} threads", threads.len());

    let scopes = m.scopes(s, top.id).await.unwrap();
    let locals = m.variables(s, scopes[0].variables_reference).await.unwrap();
    let orders = locals.iter().find(|v| v.name == "orders").expect("orders local");
    assert!(orders.variables_reference > 0);
    let children = m.variables(s, orders.variables_reference).await.unwrap();
    let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
    println!("orders children: {names:?}");
    // No type proxies: a list is its fields, `_items` holding `_size` elements and spare
    // capacity (what the collection visualiser's `backingArray` reads).
    let size = children.iter().find(|c| c.name == "_size").expect("_size field");
    assert_eq!(size.value, "3");
    let items = children.iter().find(|c| c.name == "_items").expect("_items field");
    let elements = m.variables(s, items.variables_reference).await.unwrap();
    assert_eq!(elements.first().map(|e| e.name.as_str()), Some("[0]"));
    // The visualiser's fallback for other collections: an array copy by `ToArray()`.
    let copy = m.evaluate(s, Some(top.id), "(orders).ToArray()".into()).await.expect("ToArray evaluates");
    let copied = m.variables(s, copy.variables_reference).await.unwrap();
    assert_eq!(copied.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["[0]", "[1]", "[2]"]);
    let first = &elements[0];
    let members = m.variables(s, first.variables_reference).await.unwrap();
    let total = members.iter().find(|v| v.name == "Total").expect("Total member");
    assert_eq!(total.value, "12.5");

    let caps = m.capabilities(s).unwrap();
    println!("netcoredbg gotoTargets: {}", caps.goto_targets);
    if caps.goto_targets {
        let next = line_of(DEMO_PROGRAM, "// next");
        let seen = run.seen();
        m.set_next_statement(s, thread, source.clone(), next).await.expect("set next statement");
        let _ = stopped_within(&run, seen, 10).await;
        assert_eq!(run.top(s, thread).await.line, next);
    }
    m.stop(s).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

fn dotnet_build(dir: &Path) {
    let ok = std::process::Command::new("dotnet")
        .args(["build", "-nologo", "-v:q"])
        .current_dir(dir)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "dotnet build failed in {}", dir.display());
}

/// Kills the child when dropped (a failed assertion must not leave the debuggee running).
struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// DBG-4: Attach to Process with netcoredbg (`attach` with `processId`) to a running
/// `dotnet <app>.dll`, then a breakpoint set after attaching hits.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs netcoredbg and the .NET SDK"]
async fn netcoredbg_attaches_to_a_running_process() {
    add_managed_tools_to_path();
    if !installed("netcoredbg") {
        eprintln!("netcoredbg not installed");
        return;
    }
    let dir = demo_project("attach");
    dotnet_build(&dir);
    let child = Child(
        std::process::Command::new("dotnet")
            .args(["bin/Debug/net10.0/Demo.dll", "wait"])
            .current_dir(&dir)
            .spawn()
            .unwrap(),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let run = Run::new();
    let m = &run.manager;
    let seen = run.seen();
    let s = m
        .start(netcoredbg("attach", &dir, serde_json::json!({ "processId": child.0.id() })), run.owner.clone())
        .await
        .expect("attach");
    let tick = line_of(DEMO_PROGRAM, "// tick");
    m.set_breakpoints(dir.join("Program.cs"), vec![line_bp_at(tick)]).await.unwrap();
    let (s2, thread, _) = stopped_within(&run, seen, 30).await.expect("breakpoint hit after attaching");
    assert_eq!(s2, s);
    assert_eq!(run.top(s, thread).await.line, tick);
    m.stop(s).await.unwrap();
    drop(child);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DBG-5: a Microsoft.Testing.Platform test app started with
/// `TESTINGPLATFORM_WAIT_ATTACH_DEBUGGER=1` (as the Unit Tests window's Debug does) waits;
/// netcoredbg attaches and a breakpoint in the test hits.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs netcoredbg and the .NET SDK"]
async fn netcoredbg_debugs_a_waiting_test_app() {
    add_managed_tools_to_path();
    if !installed("netcoredbg") {
        eprintln!("netcoredbg not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("openidea-netcoredbg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = real(dir);
    std::fs::write(
        dir.join("Demo.Tests.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <OutputType>Exe</OutputType>
    <EnableMSTestRunner>true</EnableMSTestRunner>
    <ImplicitUsings>enable</ImplicitUsings>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="MSTest" Version="4.5.2" />
  </ItemGroup>
</Project>
"#,
    )
    .unwrap();
    let test = r#"using Microsoft.VisualStudio.TestTools.UnitTesting;

[TestClass]
public sealed class MathTests
{
    [TestMethod]
    public void Adds()
    {
        var sum = 2 + 3; // test line
        Assert.AreEqual(5, sum);
    }
}
"#;
    std::fs::write(dir.join("MathTests.cs"), test).unwrap();
    dotnet_build(&dir);
    let child = Child(
        std::process::Command::new(dir.join("bin/Debug/net10.0/Demo.Tests"))
            .env("TESTINGPLATFORM_WAIT_ATTACH_DEBUGGER", "1")
            .current_dir(&dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let run = Run::new();
    let m = &run.manager;
    let at = line_of(test, "// test line");
    m.set_breakpoints(dir.join("MathTests.cs"), vec![line_bp_at(at)]).await.unwrap();
    let seen = run.seen();
    let s = m
        .start(netcoredbg("attach", &dir, serde_json::json!({ "processId": child.0.id() })), run.owner.clone())
        .await
        .expect("attach");
    let (_, thread, _) = stopped_within(&run, seen, 60).await.expect("breakpoint in the test hit");
    assert_eq!(run.top(s, thread).await.line, at);
    m.stop(s).await.unwrap();
    drop(child);
    let _ = std::fs::remove_dir_all(&dir);
}
