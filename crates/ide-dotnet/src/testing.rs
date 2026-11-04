//! Unit tests through Microsoft.Testing.Platform's server mode.
//!
//! Every MTP test project (MSTest, xUnit v3, NUnit with its runner, TUnit) builds
//! to an app that, started with `--server --client-host H --client-port P`,
//! connects back to us and speaks JSON-RPC with `Content-Length` framing:
//! `initialize`, then `testing/discoverTests` or `testing/runTests`, while it
//! streams `testing/testUpdates/tests` notifications (a `null` `changes` ends a
//! request), then `exit`. Nodes carry the test's type, method and, for most
//! frameworks, its source file and line, which the Unit Tests window and the
//! gutter icons need; the console output of `dotnet test` has none of that.

use ide_core::error::ErrorKind;
use ide_core::{CancellationToken, IdeError, IdeResult, IpcPath};
use ide_jsonrpc::{Handler, Peer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TestState {
    Discovered,
    InProgress,
    Passed,
    Skipped,
    Failed,
    /// The test could not run (an exception outside the assertion).
    Error,
    TimedOut,
    Cancelled,
}

impl TestState {
    fn parse(s: &str) -> Self {
        match s {
            "in-progress" => Self::InProgress,
            "passed" => Self::Passed,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            "error" => Self::Error,
            "timed-out" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            _ => Self::Discovered,
        }
    }
}

/// One test (an MTP "action" node), as discovered or with its latest result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestNode {
    /// Stable within a project; what `runTests` selects by.
    pub uid: String,
    pub display_name: String,
    /// Fully qualified type: `Fixture.Core.Tests.GreeterTests`.
    pub type_name: Option<String>,
    /// Method with its parameter types for data-driven tests: `Rejects_blank_names(System.String)`.
    pub method: Option<String>,
    pub file: Option<IpcPath>,
    /// 1-based line of the method.
    pub line: Option<u32>,
    pub state: TestState,
    pub duration_ms: Option<f64>,
    pub message: Option<String>,
    pub stack_trace: Option<String>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    /// What the test wrote to stdout / stderr.
    pub output: Option<String>,
}

/// A test project of the solution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestProject {
    pub path: IpcPath,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TestPhase {
    Building,
    Discovering,
    Running,
    Finished,
    Failed,
}

/// What a discovery or run reports as it goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestEvent {
    #[serde(rename_all = "camelCase")]
    Phase { project: Option<IpcPath>, phase: TestPhase, message: Option<String> },
    /// New or changed tests of `project`.
    #[serde(rename_all = "camelCase")]
    Tests { project: IpcPath, tests: Vec<TestNode> },
    /// Build or test-process output.
    #[serde(rename_all = "camelCase")]
    Output { project: Option<IpcPath>, line: String },
    /// The Cobertura report a coverage run of `project` wrote.
    #[serde(rename_all = "camelCase")]
    Coverage { project: IpcPath, report: IpcPath },
}

/// The tests to run in one project; empty `uids` runs all of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestSelection {
    pub project: IpcPath,
    pub uids: Vec<String>,
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned).filter(|s| !s.is_empty())
}

/// A `node` of a test update; `None` for nodes that are not tests (groups).
pub fn parse_node(node: &Value) -> Option<TestNode> {
    if node.get("node-type").and_then(Value::as_str).is_some_and(|t| t != "action") {
        return None;
    }
    let uid = str_of(node, "uid")?;
    let output = [str_of(node, "standardOutput"), str_of(node, "standardError")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    Some(TestNode {
        display_name: str_of(node, "display-name").unwrap_or_else(|| uid.clone()),
        uid,
        type_name: str_of(node, "location.type"),
        method: str_of(node, "location.method"),
        file: str_of(node, "location.file").map(IpcPath::from),
        line: node.get("location.line-start").and_then(Value::as_u64).and_then(|l| u32::try_from(l).ok()),
        state: TestState::parse(node.get("execution-state").and_then(Value::as_str).unwrap_or("")),
        duration_ms: node.get("time.duration-ms").and_then(Value::as_f64),
        message: str_of(node, "error.message"),
        stack_trace: str_of(node, "error.stacktrace"),
        expected: str_of(node, "assert.expected"),
        actual: str_of(node, "assert.actual"),
        output: (!output.is_empty()).then_some(output),
    })
}

/// The tests of a `testing/testUpdates/tests` notification; `None` when `changes`
/// is `null`, which ends the request's updates.
pub fn parse_updates(params: &Value) -> Option<Vec<TestNode>> {
    let changes = params.get("changes")?.as_array()?;
    Some(changes.iter().filter_map(|c| c.get("node")).filter_map(parse_node).collect())
}

/// Fills in the file and line of tests whose framework does not report them
/// (NUnit), by finding `class <Type>` and then `<method>(` in the project's sources.
pub fn resolve_locations(project_dir: &Path, tests: &mut [TestNode]) {
    if tests.iter().all(|t| t.file.is_some() || t.type_name.is_none()) {
        return;
    }
    let sources = source_files(project_dir);
    let mut cache: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for t in tests.iter_mut().filter(|t| t.file.is_none()) {
        let (Some(ty), Some(method)) = (&t.type_name, &t.method) else { continue };
        let class = ty.rsplit(['.', '+']).next().unwrap_or(ty).split('`').next().unwrap_or(ty);
        let method = method.split('(').next().unwrap_or(method);
        for path in &sources {
            let lines = match cache.iter().find(|(p, _)| p == path) {
                Some((_, l)) => l,
                None => {
                    let text = std::fs::read_to_string(path).unwrap_or_default();
                    cache.push((path.clone(), text.lines().map(str::to_owned).collect()));
                    &cache.last().expect("just pushed").1
                }
            };
            if let Some(line) = find_member(lines, class, method) {
                t.file = Some(IpcPath::from(path.as_path()));
                t.line = Some(line);
                break;
            }
        }
    }
}

/// 1-based line of `method` after the declaration of `class`.
fn find_member(lines: &[String], class: &str, method: &str) -> Option<u32> {
    let is_word = |s: &str, at: usize, len: usize| {
        let ok = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        ok(s[..at].chars().next_back()) && ok(s[at + len..].chars().next())
    };
    let declares_class = |l: &str| {
        let Some(at) = l.find(class) else { return false };
        is_word(l, at, class.len())
            && ["class ", "record ", "struct "].iter().any(|k| l[..at].trim_end().ends_with(k.trim_end()))
    };
    let start = lines.iter().position(|l| declares_class(l))?;
    lines.iter().enumerate().skip(start + 1).find_map(|(i, l)| {
        let at = l.find(method)?;
        let rest = l[at + method.len()..].trim_start();
        (is_word(l, at, method.len())
            && (rest.starts_with('(') || rest.starts_with('<'))
            && !l.trim_start().starts_with("//"))
        .then(|| u32::try_from(i + 1).unwrap_or(u32::MAX))
    })
}

fn source_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if p.is_dir() {
                if !matches!(name.as_ref(), "bin" | "obj" | "node_modules") && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("cs" | "fs" | "vb")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The test projects of `solution` (an `.sln`, `.slnx` or `.slnf`, else a folder's projects), by name.
pub fn test_projects(solution: Option<&Path>, dir: &Path) -> Vec<TestProject> {
    let paths = match solution.map(crate::read) {
        Some(Ok(s)) => s.projects(),
        _ => std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| crate::is_project_file(p))
            .collect(),
    };
    let mut out: Vec<TestProject> = paths
        .into_iter()
        .filter(|p| crate::read_project(p).is_ok_and(|i| i.is_test))
        .map(|p| TestProject {
            name: p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            path: IpcPath::from(p),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `dotnet msbuild <project> -getProperty:…` output: `{"Properties": {…}}`, or the
/// bare value when a single property is asked for.
pub fn parse_msbuild_properties(stdout: &str, names: &[&str]) -> Vec<(String, String)> {
    let text = stdout.trim();
    if let Some(start) = text.find('{')
        && let Ok(v) = serde_json::from_str::<Value>(&text[start..])
        && let Some(props) = v.get("Properties").and_then(Value::as_object)
    {
        return props.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect();
    }
    match names {
        [one] => vec![((*one).to_owned(), text.lines().last().unwrap_or("").trim().to_owned())],
        _ => Vec::new(),
    }
}

/// What a session does.
#[derive(Clone, Debug)]
pub enum SessionRequest {
    Discover,
    /// Runs these tests (all when empty).
    Run(Vec<String>),
}

/// How to start a project's test app.
#[derive(Clone, Debug)]
pub struct TestApp {
    /// `dotnet` (the host for `exec`).
    pub dotnet: PathBuf,
    /// The built test assembly (`TargetPath`).
    pub assembly: PathBuf,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// More options for the test app (code coverage).
    pub args: Vec<String>,
}

/// The NuGet package that collects code coverage in a Microsoft.Testing.Platform app.
pub const COVERAGE_PACKAGE: &str = "Microsoft.Testing.Extensions.CodeCoverage";

/// Whether the project in `project_dir` references [`COVERAGE_PACKAGE`] (directly or through a
/// package such as MSTest that brings it), from its restore output `obj/project.assets.json`.
pub fn has_coverage_extension(project_dir: &Path) -> bool {
    std::fs::read_to_string(project_dir.join("obj").join("project.assets.json"))
        .is_ok_and(|text| text.to_ascii_lowercase().contains(&format!("\"{}/", COVERAGE_PACKAGE.to_ascii_lowercase())))
}

/// Test-app options that write a Cobertura report to `dir/name`.
pub fn coverage_args(dir: &Path, name: &str) -> Vec<String> {
    vec![
        "--results-directory".into(),
        dir.to_string_lossy().into_owned(),
        "--coverage".into(),
        "--coverage-output-format".into(),
        "cobertura".into(),
        "--coverage-output".into(),
        name.into(),
    ]
}

struct Notifications(mpsc::UnboundedSender<(String, Value)>);

impl Handler for Notifications {
    fn on_notification(&self, method: &str, params: Value) {
        let _ = self.0.send((method.to_owned(), params));
    }
    fn on_request(&self, method: &str, _: Value) -> Result<Value, (i64, String)> {
        // `client/attachDebugger` and the like: not offered in `initialize`.
        Err((ide_jsonrpc::codes::METHOD_NOT_FOUND, format!("unsupported: {method}")))
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

fn io_error(e: impl std::fmt::Display) -> IdeError {
    IdeError::new(ErrorKind::Io, e.to_string())
}

/// Runs one discovery or run of `app` in server mode, reporting tests as they
/// change (`on_tests`) and the process's console output (`on_output`).
pub async fn session(
    app: &TestApp,
    request: SessionRequest,
    cancel: &CancellationToken,
    on_tests: impl Fn(Vec<TestNode>),
    on_output: impl Fn(String) + Send + Sync + 'static,
) -> IdeResult<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(io_error)?;
    let port = listener.local_addr().map_err(io_error)?.port();
    let mut cmd = Command::new(&app.dotnet);
    cmd.arg("exec")
        .arg(&app.assembly)
        .args(["--server", "--client-host", "127.0.0.1", "--client-port", &port.to_string()])
        .args(&app.args)
        .current_dir(&app.cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("TESTINGPLATFORM_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(root) = app.dotnet.parent().filter(|p| !p.as_os_str().is_empty()) {
        cmd.env("DOTNET_ROOT", root);
    }
    for (k, v) in &app.env {
        cmd.env(k, v);
    }
    let mut child =
        cmd.spawn().map_err(|e| IdeError::unavailable(format!("cannot start {}: {e}", app.assembly.display())))?;
    let on_output = Arc::new(on_output);
    for pipe in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as _),
    ]
    .into_iter()
    .flatten()
    {
        let out = on_output.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                out(line);
            }
        });
    }
    let init = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "DotnetIDEA", "version": env!("CARGO_PKG_VERSION") },
        "capabilities": { "testing": { "debuggerProvider": false } },
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    // An extension with a test host controller (code coverage, hang dumps) makes the app relaunch
    // itself: the controller connects first and stays passive (it cannot discover), then the test
    // host connects. The controller's connection stays open until the session ends.
    let mut controllers = Vec::new();
    let peer = loop {
        let stream = accept(&listener, &mut child, cancel).await?;
        let (reader, writer) = stream.into_split();
        let peer = Peer::connect(reader, writer, Arc::new(Notifications(tx.clone())));
        let reply =
            peer.request::<_, Value>("initialize", init.clone(), Some(cancel.clone())).await.map_err(IdeError::from)?;
        if is_host_controller(&reply) && controllers.len() < 2 {
            controllers.push(peer);
            continue;
        }
        break peer;
    };
    let run_id = format!("{:032x}", rand_id());
    let (method, params) = match &request {
        SessionRequest::Discover => ("testing/discoverTests", json!({ "runId": run_id })),
        SessionRequest::Run(uids) if uids.is_empty() => ("testing/runTests", json!({ "runId": run_id })),
        SessionRequest::Run(uids) => (
            "testing/runTests",
            json!({ "runId": run_id, "tests": uids.iter().map(|u| json!({ "uid": u, "display-name": u })).collect::<Vec<_>>() }),
        ),
    };
    let call = peer.request::<_, Value>(method, params, Some(cancel.clone()));
    tokio::pin!(call);
    let mut ended = false;
    let result = loop {
        tokio::select! {
            r = &mut call => break r.map(|_| ()).map_err(IdeError::from),
            Some((m, p)) = rx.recv() => {
                if m == "testing/testUpdates/tests" {
                    match parse_updates(&p) {
                        Some(tests) if !tests.is_empty() => on_tests(tests),
                        Some(_) => {}
                        None => ended = true,
                    }
                } else if m == "client/log"
                    && p.get("level").and_then(Value::as_str).is_some_and(|l| l == "Error" || l == "Warning")
                    && let Some(msg) = str_of(&p, "message")
                {
                    on_output(msg);
                }
            }
        }
    };
    // Updates that arrived with the response.
    while let Ok((m, p)) = rx.try_recv() {
        if m == "testing/testUpdates/tests"
            && let Some(tests) = parse_updates(&p)
            && !tests.is_empty()
        {
            on_tests(tests);
        }
    }
    let _ = ended;
    for p in controllers.iter().chain([&peer]) {
        let _ = p.notify("exit", json!({}));
    }
    if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
    result
}

/// Whether an `initialize` reply comes from a test host controller rather than the test host:
/// only the test host can discover tests.
fn is_host_controller(reply: &Value) -> bool {
    reply.pointer("/capabilities/testing/supportsDiscovery").and_then(Value::as_bool) == Some(false)
}

async fn accept(
    listener: &TcpListener,
    child: &mut tokio::process::Child,
    cancel: &CancellationToken,
) -> IdeResult<tokio::net::TcpStream> {
    tokio::select! {
        r = tokio::time::timeout(CONNECT_TIMEOUT, listener.accept()) => match r {
            Ok(Ok((s, _))) => Ok(s),
            Ok(Err(e)) => Err(io_error(e)),
            Err(_) => Err(IdeError::new(ErrorKind::Io, "the test app did not connect; is it a Microsoft.Testing.Platform project?")),
        },
        status = child.wait() => {
            let code = status.ok().and_then(|s| s.code()).map_or("?".to_owned(), |c| c.to_string());
            Err(IdeError::new(ErrorKind::Io, format!("the test app exited with code {code} before connecting")))
        }
        () = cancel.cancelled() => Err(IdeError::cancelled()),
    }
}

fn rand_id() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    t ^ (u128::from(std::process::id()) << 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_of_an_update() {
        let params = json!({"runId": "r", "changes": [
            {"node": {"uid": "a", "display-name": "Greets_by_name", "location.type": "N.GreeterTests",
              "location.method": "Greets_by_name", "location.file": "/s/T.cs", "location.line-start": 8,
              "node-type": "action", "execution-state": "passed", "time.duration-ms": 7}, "parent": null},
            {"node": {"uid": "b", "display-name": "Fails", "node-type": "action", "execution-state": "failed",
              "error.message": "Assert.Equal() Failure", "error.stacktrace": "at N.T.Fails() in /s/T.cs:line 12",
              "assert.expected": "1", "assert.actual": "2", "standardOutput": "hello"}},
            {"node": {"uid": "g", "display-name": "group", "node-type": "group"}}
        ]});
        let tests = parse_updates(&params).unwrap();
        assert_eq!(tests.len(), 2);
        let a = &tests[0];
        assert_eq!((a.state, a.line, a.duration_ms), (TestState::Passed, Some(8), Some(7.0)));
        assert_eq!(a.file.as_deref(), Some(Path::new("/s/T.cs")));
        assert_eq!(a.type_name.as_deref(), Some("N.GreeterTests"));
        let b = &tests[1];
        assert_eq!(b.state, TestState::Failed);
        assert_eq!(b.message.as_deref(), Some("Assert.Equal() Failure"));
        assert_eq!(
            (b.expected.as_deref(), b.actual.as_deref(), b.output.as_deref()),
            (Some("1"), Some("2"), Some("hello"))
        );
        assert_eq!(parse_updates(&json!({"runId": "r", "changes": null})), None);
    }

    #[test]
    fn locations_of_tests_their_framework_does_not_place() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("obj")).unwrap();
        std::fs::write(dir.path().join("obj/Gen.cs"), "class GreeterTests { void Greets_by_name() {} }").unwrap();
        std::fs::write(
            dir.path().join("UnitTest1.cs"),
            "namespace N;\n\npublic class OtherTests\n{\n    public void Greets_by_name() {}\n}\n\npublic class GreeterTests\n{\n    // Greets_by_name()\n    [Test]\n    public void Greets_by_name() { }\n\n    [TestCase(\"\")]\n    public void Rejects_blank_names(string name) { }\n}\n",
        )
        .unwrap();
        let node = |uid: &str, method: &str| TestNode {
            uid: uid.into(),
            display_name: uid.into(),
            type_name: Some("N.GreeterTests".into()),
            method: Some(method.into()),
            file: None,
            line: None,
            state: TestState::Discovered,
            duration_ms: None,
            message: None,
            stack_trace: None,
            expected: None,
            actual: None,
            output: None,
        };
        let mut tests = vec![node("a", "Greets_by_name"), node("b", "Rejects_blank_names(System.String)")];
        resolve_locations(dir.path(), &mut tests);
        assert_eq!(tests[0].file.as_deref(), Some(dir.path().join("UnitTest1.cs").as_path()));
        assert_eq!((tests[0].line, tests[1].line), (Some(12), Some(15)));
    }

    #[test]
    fn msbuild_property_output() {
        let two = "{\n  \"Properties\": {\n    \"TargetPath\": \"/s/bin/T.dll\",\n    \"IsTestingPlatformApplication\": \"true\"\n  }\n}";
        let props = parse_msbuild_properties(two, &["TargetPath", "IsTestingPlatformApplication"]);
        assert!(props.contains(&("TargetPath".into(), "/s/bin/T.dll".into())));
        assert_eq!(
            parse_msbuild_properties("/s/bin/T.dll\n", &["TargetPath"]),
            [("TargetPath".into(), "/s/bin/T.dll".into())]
        );
    }

    #[test]
    fn test_projects_of_the_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet");
        let names: Vec<String> =
            test_projects(Some(&root.join("DotnetFixture.slnx")), &root).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Fixture.Core.MSTests", "Fixture.Core.NUnitTests", "Fixture.Core.XunitTests"]);
    }

    /// Real SDK: builds the fixture's xUnit project, discovers and runs one test. Needs `dotnet`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn discovers_and_runs_the_fixture_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet").canonicalize().unwrap();
        let project = root.join("tests/Fixture.Core.XunitTests/Fixture.Core.XunitTests.csproj");
        let status =
            std::process::Command::new("dotnet").args(["build", "-nologo", "-tl:off"]).arg(&project).status().unwrap();
        assert!(status.success());
        let out = std::process::Command::new("dotnet")
            .args(["msbuild", "-getProperty:TargetPath"])
            .arg(&project)
            .output()
            .unwrap();
        let props = parse_msbuild_properties(&String::from_utf8_lossy(&out.stdout), &["TargetPath"]);
        let app = TestApp {
            dotnet: PathBuf::from("dotnet"),
            assembly: PathBuf::from(&props[0].1),
            cwd: project.parent().unwrap().to_path_buf(),
            env: Vec::new(),
            args: Vec::new(),
        };
        let cancel = CancellationToken::new();
        let found = std::sync::Mutex::new(Vec::new());
        session(&app, SessionRequest::Discover, &cancel, |t| found.lock().unwrap().extend(t), |_| {}).await.unwrap();
        let found = found.into_inner().unwrap();
        assert_eq!(found.len(), 3, "{found:?}");
        let one = found.iter().find(|t| t.method.as_deref() == Some("Greets_by_name")).unwrap();
        assert!(one.file.as_deref().is_some_and(|f| f.ends_with("UnitTest1.cs")) && one.line.is_some());
        let ran = std::sync::Mutex::new(Vec::new());
        session(&app, SessionRequest::Run(vec![one.uid.clone()]), &cancel, |t| ran.lock().unwrap().extend(t), |_| {})
            .await
            .unwrap();
        let ran = ran.into_inner().unwrap();
        let last: Vec<_> = ran.iter().filter(|t| t.state != TestState::InProgress).collect();
        assert_eq!(last.len(), 1, "only the selected test runs: {ran:?}");
        assert_eq!(last[0].state, TestState::Passed);
    }

    /// The coverage extension's host controller connects before the test host (needs `dotnet`).
    #[tokio::test]
    #[ignore = "builds the fixture with the .NET SDK"]
    async fn runs_the_fixture_tests_with_coverage() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet").canonicalize().unwrap();
        let project = root.join("tests/Fixture.Core.MSTests/Fixture.Core.MSTests.csproj");
        let status =
            std::process::Command::new("dotnet").args(["build", "-nologo", "-tl:off"]).arg(&project).status().unwrap();
        assert!(status.success());
        assert!(has_coverage_extension(project.parent().unwrap()), "MSTest brings the coverage extension");
        let out = std::process::Command::new("dotnet")
            .args(["msbuild", "-getProperty:TargetPath"])
            .arg(&project)
            .output()
            .unwrap();
        let props = parse_msbuild_properties(&String::from_utf8_lossy(&out.stdout), &["TargetPath"]);
        let dir = tempfile::tempdir().unwrap();
        let app = TestApp {
            dotnet: PathBuf::from("dotnet"),
            assembly: PathBuf::from(&props[0].1),
            cwd: project.parent().unwrap().to_path_buf(),
            env: Vec::new(),
            args: coverage_args(dir.path(), "out.cobertura.xml"),
        };
        let cancel = CancellationToken::new();
        let ran = std::sync::Mutex::new(Vec::new());
        session(&app, SessionRequest::Run(Vec::new()), &cancel, |t| ran.lock().unwrap().extend(t), |_| {})
            .await
            .unwrap();
        assert!(ran.into_inner().unwrap().iter().any(|t| t.state == TestState::Passed));
        let report = std::fs::read_to_string(dir.path().join("out.cobertura.xml")).unwrap();
        assert!(report.contains("Greeter.cs"), "{report}");
    }
}
