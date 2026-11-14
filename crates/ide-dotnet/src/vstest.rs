//! Unit tests of VSTest projects (`Microsoft.NET.Test.Sdk` with a test adapter:
//! xUnit v2, NUnit, MSTest without their Microsoft.Testing.Platform runners).
//!
//! VSTest has no server mode a client can drive without its own .NET library, so
//! discovery lists the built assembly's tests with `dotnet vstest` and a run is
//! `dotnet test --no-build` writing a TRX report, which is read when the run ends:
//! results arrive together, not one by one. A test is identified by its fully
//! qualified name; a data row that shares its method's name (xUnit, MSTest) by
//! that and its display name.

use crate::testing::{TestNode, TestState};
use ide_core::error::ErrorKind;
use ide_core::{CancellationToken, IdeError, IdeResult};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Between a data row's fully qualified name and its display name in a uid.
const ROW: char = '\u{1f}';

/// How to reach a VSTest project's tests.
#[derive(Clone, Debug)]
pub struct VsTestProject {
    pub dotnet: PathBuf,
    pub project: PathBuf,
    /// The built test assembly (`TargetPath`).
    pub assembly: PathBuf,
    /// The target framework `assembly` was built for, when the project has several.
    pub framework: Option<String>,
    pub env: BTreeMap<String, String>,
}

/// What a run of a project does besides running the tests.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Run these tests (all when empty).
    pub uids: Vec<String>,
    /// Write a Cobertura report here.
    pub coverage: Option<PathBuf>,
    /// Start the test host waiting for a debugger (`VSTEST_HOST_DEBUG`).
    pub wait_for_debugger: bool,
}

/// `Ns.Type.Method(args)` → (`Ns.Type`, `Method(args)`).
fn split_name(fqn: &str) -> (Option<&str>, &str) {
    let head = fqn.find('(').map_or(fqn, |i| &fqn[..i]);
    match head.rfind('.') {
        Some(dot) => (Some(&fqn[..dot]), &fqn[dot + 1..]),
        None => (None, fqn),
    }
}

/// A display name without its type: `Ns.Type.Method(a: 1)` → `Method(a: 1)`.
fn short_name<'a>(type_name: Option<&str>, display: &'a str) -> &'a str {
    type_name.and_then(|t| display.strip_prefix(t)).and_then(|d| d.strip_prefix('.')).unwrap_or(display)
}

fn base(name: &str) -> &str {
    name.find('(').map_or(name, |i| &name[..i]).trim_end()
}

/// The uid of the test `fqn` shown as `display`: the name itself, unless the
/// display name tells apart rows that share it.
fn uid_of(fqn: &str, display: &str) -> String {
    let (type_name, method) = split_name(fqn);
    if short_name(type_name, display) == method { fqn.to_owned() } else { format!("{fqn}{ROW}{display}") }
}

fn node(fqn: &str, display: &str) -> TestNode {
    let (type_name, method) = split_name(fqn);
    TestNode {
        uid: uid_of(fqn, display),
        display_name: display.to_owned(),
        type_name: type_name.map(str::to_owned),
        method: Some(method.to_owned()),
        file: None,
        line: None,
        state: TestState::Discovered,
        duration_ms: None,
        message: None,
        stack_trace: None,
        expected: None,
        actual: None,
        output: None,
    }
}

/// The names `--ListTests` prints: the indented lines after its heading.
pub fn parse_listed_names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .skip_while(|l| !l.trim_end().ends_with("Tests are available:"))
        .skip(1)
        .take_while(|l| l.starts_with("    "))
        .map(|l| l[4..].trim_end().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The tests of a discovery: `fqns` are the fully qualified names (one per method
/// for xUnit and MSTest, whose rows share it), `names` the display names in the
/// same order with a line per row. When the two cannot be lined up, one test per name.
pub fn discovered(fqns: &[String], names: &[String]) -> Vec<TestNode> {
    let mut fqns: Vec<&str> = fqns.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).collect();
    let mut seen = std::collections::HashSet::new();
    fqns.retain(|f| seen.insert(*f));
    let matches = |fqn: &str, name: &str| {
        let (type_name, method) = split_name(fqn);
        base(short_name(type_name, name)) == base(method)
    };
    let mut out = Vec::new();
    let mut next = 0;
    for (i, fqn) in fqns.iter().enumerate() {
        // A name with its own arguments (NUnit) is one row.
        let most = if fqn.contains('(') { 1 } else { usize::MAX };
        let start = next;
        while next < names.len() && next - start < most && matches(fqn, &names[next]) {
            next += 1;
        }
        // A custom display name matches nothing: it is this test's, unless it is the next one's.
        if next == start && next < names.len() && fqns.get(i + 1).is_none_or(|f| !matches(f, &names[next])) {
            next += 1;
        }
        if next == start {
            out.push(node(fqn, split_name(fqn).1));
        }
        out.extend(names[start..next].iter().map(|name| node(fqn, name)));
    }
    if next != names.len() {
        return fqns.iter().map(|fqn| node(fqn, split_name(fqn).1)).collect();
    }
    out
}

/// The `--filter` expression that runs `uids`; `None` runs everything.
pub fn filter_of(uids: &[String]) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for uid in uids {
        let fqn = uid.split(ROW).next().unwrap_or(uid);
        // Arguments in a name (NUnit) need escaping no shell or MSBuild property survives:
        // match the method instead, which runs its other rows as well.
        let term = match fqn.find('(') {
            Some(open) => format!("FullyQualifiedName~{}", escape_filter(&fqn[..open])),
            None => format!("FullyQualifiedName={}", escape_filter(fqn)),
        };
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    (!terms.is_empty()).then(|| terms.join("|"))
}

fn escape_filter(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '\\' | '(' | ')' | '&' | '|' | '=' | '!' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn attr(tag: &BytesStart, name: &str) -> Option<String> {
    tag.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

fn state_of(outcome: &str) -> TestState {
    match outcome {
        "Passed" | "PassedButRunAborted" | "Warning" | "Completed" => TestState::Passed,
        "Failed" => TestState::Failed,
        "Error" => TestState::Error,
        "Timeout" => TestState::TimedOut,
        "Aborted" | "Disconnected" => TestState::Cancelled,
        _ => TestState::Skipped,
    }
}

/// `00:00:01.5000000` (optionally `d.hh:mm:ss`) in milliseconds.
fn duration_ms(text: &str) -> Option<f64> {
    let mut parts = text.rsplit(':');
    let seconds: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let hours = parts.next()?;
    let (days, hours) = hours.split_once('.').unwrap_or(("0", hours));
    let hours = days.parse::<f64>().ok()? * 24.0 + hours.parse::<f64>().ok()?;
    Some(((hours * 60.0 + minutes) * 60.0 + seconds) * 1000.0)
}

#[derive(Default)]
struct TrxResult {
    test_id: String,
    name: String,
    outcome: String,
    duration: Option<f64>,
    message: String,
    stack_trace: String,
    output: String,
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim_end().to_owned();
    (!s.is_empty()).then_some(s)
}

/// Adds text to the open result's field that the innermost open element holds.
fn append_text(current: &mut Option<TrxResult>, path: &[String], s: &str) {
    let Some(r) = current.as_mut() else { return };
    let field = match path.last().map(String::as_str) {
        Some("Message") if path.iter().any(|p| p == "ErrorInfo") => &mut r.message,
        Some("StackTrace") => &mut r.stack_trace,
        Some("StdOut" | "StdErr") => &mut r.output,
        _ => return,
    };
    field.push_str(s);
}

/// The results of a TRX report, one test per result.
pub fn parse_trx(xml: &str) -> IdeResult<Vec<TestNode>> {
    let invalid = |e: &dyn std::fmt::Display| IdeError::invalid(format!("not a valid test report: {e}"));
    let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
    let mut results: Vec<TrxResult> = Vec::new();
    // Test id → (class, method).
    let mut methods: HashMap<String, (String, String)> = HashMap::new();
    let mut path: Vec<String> = Vec::new();
    let mut current: Option<TrxResult> = None;
    let mut definition: Option<String> = None;
    loop {
        let event = reader.read_event().map_err(|e| invalid(&e))?;
        match &event {
            Event::Start(tag) | Event::Empty(tag) => {
                let name = tag.local_name().as_ref().to_owned();
                match name.as_str() {
                    // Rows nested in a result (older MSTest) are results of their own.
                    "UnitTestResult" => {
                        results.extend(current.take());
                        current = Some(TrxResult {
                            test_id: attr(tag, "testId").unwrap_or_default(),
                            name: attr(tag, "testName").unwrap_or_default(),
                            outcome: attr(tag, "outcome").unwrap_or_default(),
                            duration: attr(tag, "duration").as_deref().and_then(duration_ms),
                            ..TrxResult::default()
                        });
                    }
                    "UnitTest" => definition = attr(tag, "id"),
                    "TestMethod" => {
                        if let Some(id) = &definition {
                            methods.insert(
                                id.clone(),
                                (attr(tag, "className").unwrap_or_default(), attr(tag, "name").unwrap_or_default()),
                            );
                        }
                    }
                    _ => {}
                }
                if matches!(event, Event::Start(_)) {
                    if name == "StdErr"
                        && let Some(r) = current.as_mut().filter(|r| !r.output.is_empty())
                    {
                        r.output.push('\n');
                    }
                    path.push(name);
                }
            }
            Event::Text(t) => append_text(&mut current, &path, &t.xml_content(quick_xml::XmlVersion::Implicit1_0)),
            Event::CData(t) => append_text(&mut current, &path, t),
            Event::GeneralRef(r) => {
                let ch = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => match &**r {
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "amp" => Some('&'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    },
                };
                if let Some(c) = ch {
                    append_text(&mut current, &path, c.encode_utf8(&mut [0; 4]));
                }
            }
            Event::End(_) => match path.pop().as_deref() {
                Some("Results") => results.extend(current.take()),
                Some("UnitTest") => definition = None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    results.extend(current);
    // An aggregate result with rows of its own is not a test.
    let rows: std::collections::HashSet<String> = results
        .iter()
        .filter(|r| methods.get(&r.test_id).is_some_and(|(_, m)| base(&r.name) == base(m) && r.name != *m))
        .map(|r| r.test_id.clone())
        .collect();
    let mut out = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in &results {
        *counts.entry(r.test_id.clone()).or_default() += 1;
    }
    for r in results {
        let Some((class, method)) = methods.get(&r.test_id) else { continue };
        if rows.contains(&r.test_id) && counts[&r.test_id] > 1 && r.name == *method {
            continue;
        }
        let fqn = if class.is_empty() { method.clone() } else { format!("{class}.{method}") };
        let display = if r.name.is_empty() { method.clone() } else { r.name };
        let mut test = node(&fqn, &display);
        test.state = state_of(&r.outcome);
        test.duration_ms = r.duration;
        test.message = non_empty(r.message);
        test.stack_trace = non_empty(r.stack_trace);
        test.output = non_empty(r.output);
        out.push(test);
    }
    Ok(out)
}

/// The data collector of a coverage run: coverlet when the project references it, else
/// Microsoft's, which `Microsoft.NET.Test.Sdk` brings.
pub fn coverage_collector(project_dir: &Path) -> &'static str {
    let coverlet = std::fs::read_to_string(project_dir.join("obj").join("project.assets.json"))
        .is_ok_and(|text| text.to_ascii_lowercase().contains("\"coverlet.collector/"));
    if coverlet { "XPlat Code Coverage" } else { "Code Coverage;Format=cobertura" }
}

fn io_error(e: impl std::fmt::Display) -> IdeError {
    IdeError::new(ErrorKind::Io, e.to_string())
}

fn command(project: &VsTestProject) -> Command {
    let mut cmd = Command::from(crate::sdk_command(&project.dotnet, &project.env));
    cmd.current_dir(project.project.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Runs `cmd` to its end, passing each output line to `on_line`; the exit code.
async fn run_lines(
    mut cmd: Command,
    cancel: &CancellationToken,
    on_line: impl Fn(String) + Send + Sync + 'static,
) -> IdeResult<i32> {
    let mut child = cmd.spawn().map_err(|e| IdeError::unavailable(format!("cannot run dotnet: {e}")))?;
    let on_line = Arc::new(on_line);
    let mut readers = Vec::new();
    for pipe in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as _),
    ]
    .into_iter()
    .flatten()
    {
        let out = on_line.clone();
        readers.push(tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                out(line);
            }
        }));
    }
    let status = tokio::select! {
        s = child.wait() => s.map_err(io_error)?,
        () = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(IdeError::cancelled());
        }
    };
    for r in readers {
        let _ = r.await;
    }
    Ok(status.code().unwrap_or(-1))
}

/// The tests of the project's built assembly.
pub async fn discover(
    project: &VsTestProject,
    cancel: &CancellationToken,
    on_output: impl Fn(String) + Send + Sync + Clone + 'static,
) -> IdeResult<Vec<TestNode>> {
    let list = std::env::temp_dir().join(format!("bayonet-vstest-{}-{:x}.txt", std::process::id(), stamp()));
    let mut names_cmd = command(project);
    names_cmd.arg("vstest").arg(&project.assembly).arg("--ListTests");
    let mut fqns_cmd = command(project);
    fqns_cmd
        .arg("vstest")
        .arg(&project.assembly)
        .arg("--ListFullyQualifiedTests")
        .arg(format!("--ListTestsTargetPath:{}", list.display()));
    let listed = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = listed.clone();
    let errors = on_output.clone();
    let (names, fqns) = tokio::join!(
        run_lines(names_cmd, cancel, move |line| {
            let mut text = sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            text.push_str(&line);
            text.push('\n');
        }),
        run_lines(fqns_cmd, cancel, |_| {}),
    );
    let fqns_text = std::fs::read_to_string(&list);
    let _ = std::fs::remove_file(&list);
    let listed = listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    if fqns? != 0 || names? != 0 {
        // Why it failed is in what it printed.
        listed.lines().for_each(|l| errors(l.to_owned()));
        return Err(io_error("the tests could not be listed; see the output"));
    }
    let fqns: Vec<String> = fqns_text.map_err(io_error)?.lines().map(str::to_owned).collect();
    Ok(discovered(&fqns, &parse_listed_names(&listed)))
}

/// Runs the project's tests and returns their results; `on_started` gets the test host's
/// process id when [`RunOptions::wait_for_debugger`] makes it wait for a debugger.
pub async fn run(
    project: &VsTestProject,
    options: &RunOptions,
    cancel: &CancellationToken,
    on_output: impl Fn(String) + Send + Sync + 'static,
    on_started: impl Fn(u32) + Send + Sync + 'static,
) -> IdeResult<Vec<TestNode>> {
    let results = std::env::temp_dir().join(format!("bayonet-vstest-{}-{:x}", std::process::id(), stamp()));
    let mut cmd = command(project);
    cmd.arg("test")
        .arg(&project.project)
        .args(["--no-build", "--nologo", "--logger", "trx;LogFileName=results.trx"])
        .args(["--logger", "console;verbosity=normal", "--results-directory"])
        .arg(&results);
    if let Some(framework) = &project.framework {
        cmd.args(["--framework", framework]);
    }
    if options.coverage.is_some() {
        cmd.args(["--collect", coverage_collector(project.project.parent().unwrap_or(Path::new(".")))]);
    }
    // As an environment variable the filter becomes the MSBuild property unescaped by any shell.
    if let Some(filter) = filter_of(&options.uids) {
        cmd.env("VSTestTestCaseFilter", filter);
    }
    if options.wait_for_debugger {
        cmd.env("VSTEST_HOST_DEBUG", "1");
    }
    let code = run_lines(cmd, cancel, move |line| {
        // "Process Id: 1234, Name: dotnet", once the host waits for a debugger.
        if let Some(rest) = line.trim_start().strip_prefix("Process Id:")
            && let Ok(pid) = rest.trim_start().split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse()
        {
            on_started(pid);
        }
        on_output(line);
    })
    .await;
    let report = std::fs::read_to_string(results.join("results.trx"));
    if let Some(target) = &options.coverage
        && let Some(found) = find_cobertura(&results)
    {
        if let Some(dir) = target.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::copy(found, target);
    }
    let _ = std::fs::remove_dir_all(&results);
    let code = code?;
    match report {
        Ok(xml) => parse_trx(&xml),
        // Failing tests exit with 1 and still write the report.
        Err(_) if code == 0 => Ok(Vec::new()),
        Err(_) => Err(io_error(format!("dotnet test exited with code {code}; see the output"))),
    }
}

/// The Cobertura report a collector left in a results directory (each in a folder of its own).
fn find_cobertura(results: &Path) -> Option<PathBuf> {
    std::fs::read_dir(results).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).find_map(|dir| {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("cobertura.xml")))
    })
}

fn stamp() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn names_listed_after_the_heading() {
        let out = "VSTest version 18.12.0 (arm64)\n\nThe following Tests are available:\n    Greets_by_name\n    Rejects_blank_names (\"\")\n\nDone\n";
        assert_eq!(parse_listed_names(out), ["Greets_by_name", "Rejects_blank_names (\"\")"]);
        assert!(parse_listed_names("No test is available").is_empty());
    }

    #[test]
    fn xunit_rows_share_their_methods_name() {
        let tests = discovered(
            &strings(&["N.T.Greets", "N.T.Rejects", "N.T.Last"]),
            &strings(&["N.T.Greets", "N.T.Rejects(name: \"\")", "N.T.Rejects(name: \" \")", "N.T.Last"]),
        );
        let uids: Vec<&str> = tests.iter().map(|t| t.uid.as_str()).collect();
        assert_eq!(
            uids,
            [
                "N.T.Greets",
                "N.T.Rejects\u{1f}N.T.Rejects(name: \"\")",
                "N.T.Rejects\u{1f}N.T.Rejects(name: \" \")",
                "N.T.Last"
            ]
        );
        assert_eq!(tests[1].type_name.as_deref(), Some("N.T"));
        assert_eq!(tests[1].method.as_deref(), Some("Rejects"));
        assert_eq!(tests[1].display_name, "N.T.Rejects(name: \"\")");
    }

    #[test]
    fn nunit_names_carry_their_arguments_and_mstest_rows_do_not() {
        let nunit = discovered(
            &strings(&["N.T.Greets", "N.T.Rejects(\"\")", "N.T.Rejects(\"1.5\")"]),
            &strings(&["Greets", "Rejects(\"\")", "Rejects(\"1.5\")"]),
        );
        assert_eq!(
            nunit.iter().map(|t| t.uid.as_str()).collect::<Vec<_>>(),
            ["N.T.Greets", "N.T.Rejects(\"\")", "N.T.Rejects(\"1.5\")"]
        );
        assert_eq!(nunit[2].type_name.as_deref(), Some("N.T"));
        assert_eq!(nunit[2].method.as_deref(), Some("Rejects(\"1.5\")"));
        let mstest = discovered(
            &strings(&["N.T.Greets", "N.T.Rejects"]),
            &strings(&["Greets", "Rejects (\"\")", "Rejects (\" \")"]),
        );
        assert_eq!(
            mstest.iter().map(|t| t.uid.as_str()).collect::<Vec<_>>(),
            ["N.T.Greets", "N.T.Rejects\u{1f}Rejects (\"\")", "N.T.Rejects\u{1f}Rejects (\" \")"]
        );
    }

    #[test]
    fn custom_display_names_and_lists_that_do_not_line_up() {
        let custom = discovered(&strings(&["N.T.A", "N.T.B"]), &strings(&["greets politely", "N.T.B"]));
        assert_eq!(custom[0].uid, "N.T.A\u{1f}greets politely");
        assert_eq!(custom[0].method.as_deref(), Some("A"));
        assert_eq!(custom[1].uid, "N.T.B");
        // No names, or more than can be placed: one test per fully qualified name.
        assert_eq!(discovered(&strings(&["N.T.A", "N.T.A"]), &[]).len(), 1);
        let odd = discovered(&strings(&["N.T.A"]), &strings(&["x", "y", "z"]));
        assert_eq!(odd.iter().map(|t| t.uid.as_str()).collect::<Vec<_>>(), ["N.T.A"]);
    }

    #[test]
    fn filters_select_by_fully_qualified_name() {
        assert_eq!(filter_of(&[]), None);
        let uids = strings(&[
            "N.T.Greets",
            "N.T.Rejects\u{1f}N.T.Rejects(name: \"\")",
            "N.T.Rejects\u{1f}N.T.Rejects(name: \" \")",
            "N.T.Cases(\"a|b\")",
            "N.G<T>.Run",
        ]);
        assert_eq!(
            filter_of(&uids).unwrap(),
            "FullyQualifiedName=N.T.Greets|FullyQualifiedName=N.T.Rejects|FullyQualifiedName~N.T.Cases|FullyQualifiedName=N.G<T>.Run"
        );
        assert_eq!(filter_of(&strings(&["A.B=C"])).unwrap(), "FullyQualifiedName=A.B\\=C");
    }

    const TRX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<TestRun id="1" xmlns="http://microsoft.com/schemas/VisualStudio/TeamTest/2010">
  <Results>
    <UnitTestResult testId="a" testName="N.T.Rejects(name: &quot;&quot;)" duration="00:00:00.0029377" outcome="Passed" />
    <UnitTestResult testId="b" testName="N.T.Rejects(name: &quot; &quot;)" duration="00:00:01.5000000" outcome="Passed">
      <Output><StdOut>hello &lt;world&gt;</StdOut></Output>
    </UnitTestResult>
    <UnitTestResult testId="c" testName="N.T.Skipped" outcome="NotExecuted">
      <Output><ErrorInfo><Message>not ready</Message></ErrorInfo></Output>
    </UnitTestResult>
    <UnitTestResult testId="d" testName="N.T.Fails" duration="00:00:00.0010000" outcome="Failed">
      <Output>
        <ErrorInfo>
          <Message>Assert.True() Failure</Message>
          <StackTrace>   at N.T.Fails() in /src/T.cs:line 19</StackTrace>
        </ErrorInfo>
      </Output>
    </UnitTestResult>
  </Results>
  <TestDefinitions>
    <UnitTest name="N.T.Rejects(name: &quot;&quot;)" id="a"><TestMethod className="N.T" name="Rejects" /></UnitTest>
    <UnitTest name="N.T.Rejects(name: &quot; &quot;)" id="b"><TestMethod className="N.T" name="Rejects" /></UnitTest>
    <UnitTest name="N.T.Skipped" id="c"><TestMethod className="N.T" name="Skipped" /></UnitTest>
    <UnitTest name="N.T.Fails" id="d"><TestMethod className="N.T" name="Fails" /></UnitTest>
  </TestDefinitions>
  <ResultSummary outcome="Failed"><Output><StdOut>run output</StdOut></Output></ResultSummary>
</TestRun>"#;

    #[test]
    fn results_of_a_report() {
        let tests = parse_trx(TRX).unwrap();
        assert_eq!(tests.len(), 4);
        assert_eq!(tests[0].uid, "N.T.Rejects\u{1f}N.T.Rejects(name: \"\")");
        assert_eq!(tests[0].state, TestState::Passed);
        assert!((tests[0].duration_ms.unwrap() - 2.9377).abs() < 1e-9);
        assert_eq!(tests[1].duration_ms, Some(1500.0));
        assert_eq!(tests[1].output.as_deref(), Some("hello <world>"));
        assert_eq!((tests[2].uid.as_str(), tests[2].state), ("N.T.Skipped", TestState::Skipped));
        assert_eq!(tests[2].message.as_deref(), Some("not ready"));
        assert_eq!(tests[3].state, TestState::Failed);
        assert_eq!(tests[3].message.as_deref(), Some("Assert.True() Failure"));
        assert_eq!(tests[3].stack_trace.as_deref(), Some("   at N.T.Fails() in /src/T.cs:line 19"));
        assert_eq!(tests[3].output, None, "the run's output is not a test's");
    }

    #[test]
    fn rows_nested_in_a_result_replace_it() {
        let trx = r#"<TestRun><Results>
            <UnitTestResult testId="a" testName="Rejects" outcome="Failed"><InnerResults>
              <UnitTestResult testId="a" testName="Rejects (1)" outcome="Passed" />
              <UnitTestResult testId="a" testName="Rejects (2)" outcome="Failed" />
            </InnerResults></UnitTestResult>
          </Results>
          <TestDefinitions><UnitTest id="a"><TestMethod className="N.T" name="Rejects" /></UnitTest></TestDefinitions>
        </TestRun>"#;
        let tests = parse_trx(trx).unwrap();
        assert_eq!(
            tests.iter().map(|t| (t.uid.as_str(), t.state)).collect::<Vec<_>>(),
            [("N.T.Rejects\u{1f}Rejects (1)", TestState::Passed), ("N.T.Rejects\u{1f}Rejects (2)", TestState::Failed)]
        );
    }

    #[test]
    fn durations() {
        assert_eq!(duration_ms("00:01:02.5"), Some(62_500.0));
        assert_eq!(duration_ms("1.01:00:00"), Some(25.0 * 3_600_000.0));
        assert_eq!(duration_ms("soon"), None);
    }

    fn fixture(project: &str) -> VsTestProject {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet-vstest").canonicalize().unwrap();
        let path = root.join(format!("tests/{project}/{project}.csproj"));
        let status =
            std::process::Command::new("dotnet").args(["build", "-nologo", "-tl:off"]).arg(&path).status().unwrap();
        assert!(status.success());
        VsTestProject {
            dotnet: PathBuf::from("dotnet"),
            assembly: root.join(format!("tests/{project}/bin/Debug/net10.0/{project}.dll")),
            project: path,
            framework: None,
            env: BTreeMap::new(),
        }
    }

    /// Real SDK (TST-1): every test of the three VSTest frameworks is found, rows included.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "builds the VSTest fixture with the .NET SDK"]
    async fn discovers_the_vstest_fixture() {
        let cancel = CancellationToken::new();
        for (project, expected) in
            [("Legacy.Core.XunitTests", 5), ("Legacy.Core.NUnitTests", 3), ("Legacy.Core.MSTests", 3)]
        {
            let tests = discover(&fixture(project), &cancel, |_| {}).await.unwrap();
            assert_eq!(tests.len(), expected, "{project}: {tests:?}");
            let rows = tests.iter().filter(|t| t.method.as_deref().is_some_and(|m| base(m) == "Rejects_blank_names"));
            assert_eq!(rows.count(), 2, "{project}: {tests:?}");
            assert!(tests.iter().all(|t| t.type_name.as_deref().is_some_and(|n| n.ends_with(".GreeterTests"))));
        }
    }

    /// Real SDK (TST-1, TST-4): a selected test runs alone, and a whole run reports each
    /// outcome and writes coverage, with either collector.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "builds the VSTest fixture with the .NET SDK"]
    async fn runs_the_vstest_fixture_with_coverage() {
        let cancel = CancellationToken::new();
        let xunit = fixture("Legacy.Core.XunitTests");
        let found = discover(&xunit, &cancel, |_| {}).await.unwrap();
        let one = found.iter().find(|t| t.method.as_deref() == Some("Greets_by_name")).unwrap();
        let options = RunOptions { uids: vec![one.uid.clone()], ..RunOptions::default() };
        let ran = run(&xunit, &options, &cancel, |_| {}, |_| {}).await.unwrap();
        assert_eq!(
            ran.iter().map(|t| (t.uid.as_str(), t.state)).collect::<Vec<_>>(),
            [(one.uid.as_str(), TestState::Passed)]
        );

        let dir = tempfile::tempdir().unwrap();
        for (project, collector) in [
            ("Legacy.Core.XunitTests", "XPlat Code Coverage"),
            ("Legacy.Core.MSTests", "Code Coverage;Format=cobertura"),
        ] {
            let project = fixture(project);
            assert_eq!(coverage_collector(project.project.parent().unwrap()), collector);
            let report = dir.path().join(format!("{collector}.xml"));
            let options = RunOptions { coverage: Some(report.clone()), ..RunOptions::default() };
            let ran = run(&project, &options, &cancel, |_| {}, |_| {}).await.unwrap();
            let found = discover(&project, &cancel, |_| {}).await.unwrap();
            // A run names its tests as discovery did.
            for t in &ran {
                assert!(found.iter().any(|f| f.uid == t.uid), "{} is not among {found:?}", t.uid);
            }
            assert_eq!(ran.len(), found.len());
            assert!(ran.iter().any(|t| t.state == TestState::Passed));
            let xml = std::fs::read_to_string(&report).unwrap();
            assert!(xml.contains("Greeter.cs"), "{xml}");
        }
        let skipped = found.iter().find(|t| t.method.as_deref() == Some("Is_skipped")).unwrap();
        let mut failing = xunit.clone();
        failing.env.insert("LEGACY_FAIL".into(), "1".into());
        let ran = run(&failing, &RunOptions::default(), &cancel, |_| {}, |_| {}).await.unwrap();
        let state = |uid: &str| ran.iter().find(|t| t.uid == uid).map(|t| t.state);
        assert_eq!(state(&skipped.uid), Some(TestState::Skipped));
        let failed = ran.iter().find(|t| t.state == TestState::Failed).expect("LEGACY_FAIL fails a test");
        assert_eq!(failed.method.as_deref(), Some("Fails_on_request"));
        assert!(failed.message.as_deref().is_some_and(|m| m.contains("LEGACY_FAIL is set")));
        assert!(failed.stack_trace.as_deref().is_some_and(|s| s.contains("GreeterTests.cs")));
    }

    /// Real SDK (DBG-5): a debug run reports the test host's process id and waits for a debugger.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "builds the VSTest fixture with the .NET SDK"]
    async fn a_debug_run_reports_the_waiting_test_host() {
        let project = fixture("Legacy.Core.MSTests");
        let cancel = CancellationToken::new();
        let pid = Arc::new(std::sync::Mutex::new(None));
        let (seen, stop) = (pid.clone(), cancel.clone());
        let options = RunOptions { wait_for_debugger: true, ..RunOptions::default() };
        let result = run(
            &project,
            &options,
            &cancel,
            |_| {},
            move |p| {
                *seen.lock().unwrap() = Some(p);
                stop.cancel();
            },
        )
        .await;
        assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
        let pid = pid.lock().unwrap().expect("no process id");
        // The host outlives the cancelled `dotnet test`: end it.
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }
}
