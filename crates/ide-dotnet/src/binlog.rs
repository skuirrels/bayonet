//! Reader of MSBuild binary logs (`.binlog`, BLD-3): the build as a tree of
//! projects, their targets and tasks with durations, and the errors, warnings
//! and messages logged in each, for the binary log viewer.
//!
//! The format is MSBuild's own (`src/Build/Logging/BinaryLogger` in dotnet/msbuild):
//! a gzip stream of a 4-byte version, a 4-byte minimum reader version, then records
//! of a 7-bit encoded kind. Since version 18 every event record is preceded by its
//! length, so records this reader does not need are skipped whole. Strings and
//! name-value lists are deduplicated into their own records and referred to by index.

use flate2::read::GzDecoder;
use ide_core::{IdeError, IdeResult};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// Oldest format with length-prefixed records (MSBuild 17.8, .NET 8 SDK).
const MIN_VERSION: i32 = 18;
/// Messages kept per node and in all; a big build logs millions.
const NODE_MESSAGES: usize = 2_000;
const TOTAL_MESSAGES: usize = 200_000;
/// Longer messages are cut (whole item lists, response files).
const MESSAGE_CHARS: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BinlogSeverity {
    Error,
    Warning,
    /// `MessageImportance.High` and critical messages.
    High,
    Normal,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BinlogMessage {
    pub severity: BinlogSeverity,
    pub text: String,
    pub code: Option<String>,
    pub file: Option<String>,
    /// 1-based, 0 when unknown.
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BinlogTask {
    pub name: String,
    pub start_ms: u32,
    pub duration_ms: u32,
    pub succeeded: bool,
    pub command_line: Option<String>,
    pub messages: Vec<BinlogMessage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BinlogTarget {
    pub name: String,
    pub file: Option<String>,
    /// The target that made this one run (`DependsOnTargets`, `BeforeTargets`, …).
    pub parent: Option<String>,
    pub start_ms: u32,
    pub duration_ms: u32,
    pub succeeded: bool,
    /// Why it did not run (`condition false`, `outputs up to date`, …); `None` when it ran.
    pub skipped: Option<String>,
    pub tasks: Vec<BinlogTask>,
    pub messages: Vec<BinlogMessage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BinlogProject {
    /// MSBuild's project context id (unique in the build).
    pub id: i32,
    /// The project that built this one (`MSBuild` task, solution metaproject).
    pub parent: Option<i32>,
    pub file: String,
    /// `Fixture.Api.csproj`.
    pub name: String,
    pub target_framework: Option<String>,
    pub configuration: Option<String>,
    /// Targets requested (empty: default targets).
    pub entry_targets: String,
    pub start_ms: u32,
    pub duration_ms: u32,
    pub succeeded: bool,
    pub errors: u32,
    pub warnings: u32,
    pub targets: Vec<BinlogTarget>,
    pub messages: Vec<BinlogMessage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Binlog {
    pub format_version: i32,
    pub succeeded: bool,
    pub cancelled: bool,
    pub duration_ms: u32,
    /// Projects in start order.
    pub projects: Vec<BinlogProject>,
    /// Errors and warnings of the whole build, in logged order.
    pub diagnostics: Vec<BinlogMessage>,
    /// Messages outside any project (evaluation, the build itself).
    pub messages: Vec<BinlogMessage>,
    /// Messages were left out (limits per node or in all).
    pub truncated: bool,
}

/// Reads the binary log at `path`.
pub fn read(path: &Path) -> IdeResult<Binlog> {
    let file =
        std::fs::File::open(path).map_err(|e| IdeError::not_found(format!("cannot open {}: {e}", path.display())))?;
    let mut bytes = Vec::new();
    GzDecoder::new(std::io::BufReader::new(file))
        .read_to_end(&mut bytes)
        .map_err(|e| IdeError::invalid(format!("{} is not a binary log: {e}", path.display())))?;
    parse(&bytes)
}

// ---------------------------------------------------------------- primitives

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

fn eof() -> IdeError {
    IdeError::invalid("the binary log ends unexpectedly")
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cursor { b, i: 0 }
    }

    fn done(&self) -> bool {
        self.i >= self.b.len()
    }

    fn bytes(&mut self, n: usize) -> IdeResult<&'a [u8]> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or_else(eof)?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }

    fn u8(&mut self) -> IdeResult<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn bool(&mut self) -> IdeResult<bool> {
        Ok(self.u8()? != 0)
    }

    fn i32_le(&mut self) -> IdeResult<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().map_err(|_| eof())?))
    }

    fn i64_le(&mut self) -> IdeResult<i64> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().map_err(|_| eof())?))
    }

    /// .NET's `Read7BitEncodedInt`.
    fn int(&mut self) -> IdeResult<i32> {
        let mut value: u32 = 0;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            value |= u32::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(IdeError::invalid("bad 7-bit integer in the binary log"))
    }

    fn len(&mut self) -> IdeResult<usize> {
        usize::try_from(self.int()?).map_err(|_| IdeError::invalid("negative length in the binary log"))
    }

    /// `BinaryWriter.Write(string)`: 7-bit byte length, UTF-8.
    fn string(&mut self) -> IdeResult<String> {
        let n = self.len()?;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Ctx {
    project: i32,
    target: i32,
    task: i32,
}

const INVALID_PROJECT: i32 = -2;
const INVALID_ID: i32 = -1;

// Field flags (`BuildEventArgsFieldFlags`).
const F_CONTEXT: i32 = 1 << 0;
const F_HELP: i32 = 1 << 1;
const F_MESSAGE: i32 = 1 << 2;
const F_SENDER: i32 = 1 << 3;
const F_THREAD: i32 = 1 << 4;
const F_TIMESTAMP: i32 = 1 << 5;
const F_SUBCATEGORY: i32 = 1 << 6;
const F_CODE: i32 = 1 << 7;
const F_FILE: i32 = 1 << 8;
const F_PROJECT_FILE: i32 = 1 << 9;
const F_LINE: i32 = 1 << 10;
const F_COLUMN: i32 = 1 << 11;
const F_END_LINE: i32 = 1 << 12;
const F_END_COLUMN: i32 = 1 << 13;
const F_ARGUMENTS: i32 = 1 << 14;
const F_IMPORTANCE: i32 = 1 << 15;
const F_EXTENDED: i32 = 1 << 16;

#[derive(Default)]
struct Fields {
    message: Option<String>,
    ctx: Ctx,
    has_ctx: bool,
    timestamp: Option<i64>,
    code: Option<String>,
    file: Option<String>,
    line: i32,
    col: i32,
    arguments: Vec<String>,
    importance: i32,
}

// Record kinds (`BinaryLogRecordKind`).
const K_END: i32 = 0;
const K_BUILD_STARTED: i32 = 1;
const K_BUILD_FINISHED: i32 = 2;
const K_PROJECT_STARTED: i32 = 3;
const K_PROJECT_FINISHED: i32 = 4;
const K_TARGET_STARTED: i32 = 5;
const K_TARGET_FINISHED: i32 = 6;
const K_TASK_STARTED: i32 = 7;
const K_TASK_FINISHED: i32 = 8;
const K_ERROR: i32 = 9;
const K_WARNING: i32 = 10;
const K_MESSAGE: i32 = 11;
const K_COMMAND_LINE: i32 = 12;
const K_CRITICAL: i32 = 13;
const K_IMPORT_ARCHIVE: i32 = 17;
const K_TARGET_SKIPPED: i32 = 18;
const K_NAME_VALUE_LIST: i32 = 23;
const K_STRING: i32 = 24;
const K_BUILD_CANCELED: i32 = 34;

/// .NET ticks (100 ns) to whole milliseconds (negative spans are 0).
fn ms(ticks: i64) -> u32 {
    u32::try_from((ticks / 10_000).max(0)).unwrap_or(u32::MAX)
}

/// `String.Format` of `{0}`, `{1:N0}`, `{2,-10}` with `{{` / `}}` escapes (formats are ignored).
fn format_message(template: &str, args: &[String]) -> String {
    if args.is_empty() {
        return template.to_owned();
    }
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut spec = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    spec.push(d);
                }
                let index = spec.split([',', ':']).next().and_then(|n| n.trim().parse::<usize>().ok());
                match index.and_then(|i| args.get(i)) {
                    Some(a) => out.push_str(a),
                    None => {
                        out.push('{');
                        out.push_str(&spec);
                        out.push('}');
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn clip(mut text: String) -> String {
    if text.chars().count() > MESSAGE_CHARS {
        text = text.chars().take(MESSAGE_CHARS).collect();
        text.push('…');
    }
    text
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

fn skip_reason(reason: i32, condition: Option<&str>) -> String {
    match reason {
        1 => "previously built successfully".to_owned(),
        2 => "previously built unsuccessfully".to_owned(),
        3 => "outputs up to date".to_owned(),
        4 => match condition {
            Some(c) => format!("condition false: {c}"),
            None => "condition false".to_owned(),
        },
        _ => "skipped".to_owned(),
    }
}

// ---------------------------------------------------------------- reader

struct Reader {
    version: i32,
    strings: Vec<String>,
    lists: Vec<Vec<(i32, i32)>>,
    start: Option<i64>,
    end: Option<i64>,
    succeeded: bool,
    cancelled: bool,
    projects: Vec<BinlogProject>,
    project_index: HashMap<i32, usize>,
    project_start: HashMap<i32, i64>,
    /// (project, target id) → index in the project's targets, start ticks.
    targets: HashMap<(i32, i32), (usize, i64)>,
    /// (project, target id, task id) → index in the target's tasks, start ticks.
    tasks: HashMap<(i32, i32, i32), (usize, i64)>,
    diagnostics: Vec<BinlogMessage>,
    messages: Vec<BinlogMessage>,
    kept: usize,
    truncated: bool,
}

impl Reader {
    fn string_at(&self, index: i32) -> Option<String> {
        match index {
            0 => None,
            1 => Some(String::new()),
            i => self.strings.get(usize::try_from(i - 10).ok()?).cloned(),
        }
    }

    fn dedup(&self, c: &mut Cursor) -> IdeResult<Option<String>> {
        let i = c.int()?;
        Ok(self.string_at(i))
    }

    fn dictionary(&self, c: &mut Cursor) -> IdeResult<HashMap<String, String>> {
        let index = c.int()?;
        let Some(list) = usize::try_from(index - 10).ok().and_then(|i| self.lists.get(i)) else {
            return Ok(HashMap::new());
        };
        Ok(list
            .iter()
            .filter_map(|(k, v)| Some((self.string_at(*k)?, self.string_at(*v).unwrap_or_default())))
            .collect())
    }

    fn context(&self, c: &mut Cursor) -> IdeResult<Ctx> {
        let _node = c.int()?;
        let project = c.int()?;
        let target = c.int()?;
        let task = c.int()?;
        let _submission = c.int()?;
        let _instance = c.int()?;
        let _evaluation = c.int()?;
        Ok(Ctx { project, target, task })
    }

    fn fields(&self, c: &mut Cursor) -> IdeResult<Fields> {
        let flags = c.int()?;
        let mut f = Fields { importance: 1, ..Fields::default() };
        if flags & F_MESSAGE != 0 {
            f.message = self.dedup(c)?;
        }
        if flags & F_CONTEXT != 0 {
            f.ctx = self.context(c)?;
            f.has_ctx = true;
        }
        if flags & F_THREAD != 0 {
            c.int()?;
        }
        if flags & F_HELP != 0 {
            self.dedup(c)?;
        }
        if flags & F_SENDER != 0 {
            self.dedup(c)?;
        }
        if flags & F_TIMESTAMP != 0 {
            f.timestamp = Some(c.i64_le()?);
            c.int()?; // DateTimeKind
        }
        if flags & F_EXTENDED != 0 {
            self.dedup(c)?; // type
            c.int()?; // metadata (name-value list)
            self.dedup(c)?; // data
        }
        if flags & F_SUBCATEGORY != 0 {
            self.dedup(c)?;
        }
        if flags & F_CODE != 0 {
            f.code = self.dedup(c)?;
        }
        if flags & F_FILE != 0 {
            f.file = self.dedup(c)?;
        }
        if flags & F_PROJECT_FILE != 0 {
            self.dedup(c)?;
        }
        if flags & F_LINE != 0 {
            f.line = c.int()?;
        }
        if flags & F_COLUMN != 0 {
            f.col = c.int()?;
        }
        if flags & F_END_LINE != 0 {
            c.int()?;
        }
        if flags & F_END_COLUMN != 0 {
            c.int()?;
        }
        if flags & F_ARGUMENTS != 0 {
            let n = c.len()?;
            for _ in 0..n {
                f.arguments.push(self.dedup(c)?.unwrap_or_default());
            }
        }
        if flags & F_IMPORTANCE != 0 {
            f.importance = c.int()?;
        }
        Ok(f)
    }

    fn rel(&self, ticks: Option<i64>) -> u32 {
        match (ticks, self.start) {
            (Some(t), Some(s)) => ms(t - s),
            _ => 0,
        }
    }

    fn text(f: &Fields) -> String {
        clip(format_message(f.message.as_deref().unwrap_or_default(), &f.arguments))
    }

    /// Files `m` under the task, target or project of `ctx`, else the build.
    fn add_message(&mut self, ctx: Option<Ctx>, m: BinlogMessage) {
        let diagnostic = matches!(m.severity, BinlogSeverity::Error | BinlogSeverity::Warning);
        if !diagnostic {
            if self.kept >= TOTAL_MESSAGES {
                self.truncated = true;
                return;
            }
            self.kept += 1;
        }
        let truncated = &mut self.truncated;
        let push = |list: &mut Vec<BinlogMessage>, m: BinlogMessage, truncated: &mut bool| {
            if diagnostic || list.len() < NODE_MESSAGES {
                list.push(m);
            } else {
                *truncated = true;
            }
        };
        let Some(ctx) = ctx.filter(|c| c.project != INVALID_PROJECT) else {
            push(&mut self.messages, m, truncated);
            return;
        };
        let Some(&p) = self.project_index.get(&ctx.project) else {
            push(&mut self.messages, m, truncated);
            return;
        };
        let project = &mut self.projects[p];
        if diagnostic {
            match m.severity {
                BinlogSeverity::Error => project.errors += 1,
                _ => project.warnings += 1,
            }
        }
        if ctx.target != INVALID_ID
            && let Some(&(t, _)) = self.targets.get(&(ctx.project, ctx.target))
        {
            let target = &mut project.targets[t];
            if ctx.task != INVALID_ID
                && let Some(&(k, _)) = self.tasks.get(&(ctx.project, ctx.target, ctx.task))
            {
                push(&mut target.tasks[k].messages, m, truncated);
            } else {
                push(&mut target.messages, m, truncated);
            }
            return;
        }
        push(&mut project.messages, m, truncated);
    }

    fn record(&mut self, kind: i32, c: &mut Cursor) -> IdeResult<()> {
        match kind {
            K_BUILD_STARTED => {
                let f = self.fields(c)?;
                self.start = f.timestamp;
            }
            K_BUILD_FINISHED => {
                let f = self.fields(c)?;
                self.succeeded = c.bool()?;
                self.end = f.timestamp;
            }
            K_BUILD_CANCELED => {
                let f = self.fields(c)?;
                self.cancelled = true;
                self.end = self.end.or(f.timestamp);
            }
            K_PROJECT_STARTED => {
                let f = self.fields(c)?;
                let parent = if c.bool()? { Some(self.context(c)?.project) } else { None };
                let file = self.dedup(c)?.unwrap_or_default();
                let _project_id = c.int()?;
                let entry_targets = self.dedup(c)?.unwrap_or_default();
                let _tools_version = self.dedup(c)?;
                let globals = self.dictionary(c)?;
                let get = |k: &str| globals.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
                let id = f.ctx.project;
                self.project_index.insert(id, self.projects.len());
                if let Some(t) = f.timestamp {
                    self.project_start.insert(id, t);
                }
                self.projects.push(BinlogProject {
                    id,
                    parent: parent.filter(|p| *p != INVALID_PROJECT),
                    name: file_name(&file),
                    file,
                    target_framework: get("TargetFramework"),
                    configuration: get("Configuration"),
                    entry_targets,
                    start_ms: self.rel(f.timestamp),
                    duration_ms: 0,
                    succeeded: false,
                    errors: 0,
                    warnings: 0,
                    targets: Vec::new(),
                    messages: Vec::new(),
                });
            }
            K_PROJECT_FINISHED => {
                let f = self.fields(c)?;
                let _file = self.dedup(c)?;
                let succeeded = c.bool()?;
                if let Some(&p) = self.project_index.get(&f.ctx.project) {
                    let start = self.project_start.get(&f.ctx.project).copied();
                    let project = &mut self.projects[p];
                    project.succeeded = succeeded;
                    if let (Some(s), Some(e)) = (start, f.timestamp) {
                        project.duration_ms = ms(e - s);
                    }
                }
            }
            K_TARGET_STARTED => {
                let f = self.fields(c)?;
                let name = self.dedup(c)?.unwrap_or_default();
                let _project_file = self.dedup(c)?;
                let file = self.dedup(c)?;
                let parent = self.dedup(c)?.filter(|p| !p.is_empty());
                let start_ms = self.rel(f.timestamp);
                if let Some(&p) = self.project_index.get(&f.ctx.project) {
                    let targets = &mut self.projects[p].targets;
                    self.targets.insert((f.ctx.project, f.ctx.target), (targets.len(), f.timestamp.unwrap_or(0)));
                    targets.push(BinlogTarget {
                        name,
                        file,
                        parent,
                        start_ms,
                        duration_ms: 0,
                        succeeded: false,
                        skipped: None,
                        tasks: Vec::new(),
                        messages: Vec::new(),
                    });
                }
            }
            K_TARGET_FINISHED => {
                let f = self.fields(c)?;
                let succeeded = c.bool()?;
                let key = (f.ctx.project, f.ctx.target);
                if let (Some(&p), Some(&(t, start))) = (self.project_index.get(&f.ctx.project), self.targets.get(&key))
                {
                    let target = &mut self.projects[p].targets[t];
                    target.succeeded = succeeded;
                    if let Some(e) = f.timestamp {
                        target.duration_ms = ms(e - start);
                    }
                }
            }
            K_TARGET_SKIPPED => {
                let f = self.fields(c)?;
                let file = self.dedup(c)?;
                let name = self.dedup(c)?.unwrap_or_default();
                let parent = self.dedup(c)?.filter(|p| !p.is_empty());
                let condition = self.dedup(c)?;
                let _evaluated = self.dedup(c)?;
                let originally = c.bool()?;
                let _reason = c.int()?;
                let skip = if self.version >= 14 {
                    c.int()?
                } else if originally {
                    1
                } else {
                    2
                };
                if let Some(&p) = self.project_index.get(&f.ctx.project) {
                    let start_ms = self.rel(f.timestamp);
                    self.projects[p].targets.push(BinlogTarget {
                        name,
                        file,
                        parent,
                        start_ms,
                        duration_ms: 0,
                        succeeded: true,
                        skipped: Some(skip_reason(skip, condition.as_deref())),
                        tasks: Vec::new(),
                        messages: Vec::new(),
                    });
                }
            }
            K_TASK_STARTED => {
                let f = self.fields(c)?;
                let name = self.dedup(c)?.unwrap_or_default();
                let start_ms = self.rel(f.timestamp);
                let key = (f.ctx.project, f.ctx.target);
                if let (Some(&p), Some(&(t, _))) = (self.project_index.get(&f.ctx.project), self.targets.get(&key)) {
                    let tasks = &mut self.projects[p].targets[t].tasks;
                    self.tasks
                        .insert((f.ctx.project, f.ctx.target, f.ctx.task), (tasks.len(), f.timestamp.unwrap_or(0)));
                    tasks.push(BinlogTask {
                        name,
                        start_ms,
                        duration_ms: 0,
                        succeeded: false,
                        command_line: None,
                        messages: Vec::new(),
                    });
                }
            }
            K_TASK_FINISHED => {
                let f = self.fields(c)?;
                let succeeded = c.bool()?;
                if let Some(task) = self.task_mut(f.ctx) {
                    task.0.succeeded = succeeded;
                    if let Some(e) = f.timestamp {
                        task.0.duration_ms = ms(e - task.1);
                    }
                }
            }
            K_COMMAND_LINE => {
                let f = self.fields(c)?;
                let command = self.dedup(c)?;
                if let Some((task, _)) = self.task_mut(f.ctx) {
                    task.command_line = command.map(clip);
                }
            }
            K_ERROR | K_WARNING => {
                let f = self.fields(c)?;
                // The diagnostic fields again, explicitly (subcategory, code, file, project file, positions).
                let _subcategory = self.dedup(c)?;
                let code = self.dedup(c)?;
                let file = self.dedup(c)?;
                let _project_file = self.dedup(c)?;
                let line = c.int()?;
                let col = c.int()?;
                let m = BinlogMessage {
                    severity: if kind == K_ERROR { BinlogSeverity::Error } else { BinlogSeverity::Warning },
                    text: Self::text(&f),
                    code: code.or(f.code.clone()),
                    file: file.or(f.file.clone()),
                    line: line.max(0) as u32,
                    col: col.max(0) as u32,
                };
                self.diagnostics.push(m.clone());
                self.add_message(f.has_ctx.then_some(f.ctx), m);
            }
            K_MESSAGE | K_CRITICAL => {
                let f = self.fields(c)?;
                let severity = match (kind, f.importance) {
                    (K_CRITICAL, _) | (_, 0) => BinlogSeverity::High,
                    (_, 1) => BinlogSeverity::Normal,
                    _ => BinlogSeverity::Low,
                };
                let m = BinlogMessage {
                    severity,
                    text: Self::text(&f),
                    code: f.code.clone(),
                    file: f.file.clone(),
                    line: f.line.max(0) as u32,
                    col: f.col.max(0) as u32,
                };
                self.add_message(f.has_ctx.then_some(f.ctx), m);
            }
            _ => {}
        }
        Ok(())
    }

    fn task_mut(&mut self, ctx: Ctx) -> Option<(&mut BinlogTask, i64)> {
        let &p = self.project_index.get(&ctx.project)?;
        let &(t, _) = self.targets.get(&(ctx.project, ctx.target))?;
        let &(k, start) = self.tasks.get(&(ctx.project, ctx.target, ctx.task))?;
        Some((&mut self.projects[p].targets[t].tasks[k], start))
    }
}

/// Parses the decompressed bytes of a binary log.
pub fn parse(bytes: &[u8]) -> IdeResult<Binlog> {
    let mut c = Cursor::new(bytes);
    let version = c.i32_le()?;
    if version < MIN_VERSION {
        return Err(IdeError::invalid(format!(
            "binary log format {version} is too old (made by MSBuild before 17.8); rebuild with a newer .NET SDK"
        )));
    }
    let _min_reader = c.i32_le()?;
    let mut r = Reader {
        version,
        strings: Vec::new(),
        lists: Vec::new(),
        start: None,
        end: None,
        succeeded: false,
        cancelled: false,
        projects: Vec::new(),
        project_index: HashMap::new(),
        project_start: HashMap::new(),
        targets: HashMap::new(),
        tasks: HashMap::new(),
        diagnostics: Vec::new(),
        messages: Vec::new(),
        kept: 0,
        truncated: false,
    };
    while !c.done() {
        let kind = c.int()?;
        match kind {
            K_END => break,
            K_STRING => {
                let s = c.string()?;
                r.strings.push(s);
            }
            K_NAME_VALUE_LIST => {
                let n = c.len()?;
                let mut body = Cursor::new(c.bytes(n)?);
                let count = body.len()?;
                let mut list = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    list.push((body.int()?, body.int()?));
                }
                r.lists.push(list);
            }
            K_IMPORT_ARCHIVE => {
                let n = c.len()?;
                c.bytes(n)?;
            }
            _ => {
                let n = c.len()?;
                let mut body = Cursor::new(c.bytes(n)?);
                // A record this reader misreads is skipped, not fatal: its length is known.
                if let Err(e) = r.record(kind, &mut body) {
                    tracing::debug!("binlog: record kind {kind} skipped: {}", e.message);
                }
            }
        }
    }
    let duration_ms = match (r.start, r.end) {
        (Some(s), Some(e)) => ms(e - s),
        _ => r.projects.iter().map(|p| p.start_ms.saturating_add(p.duration_ms)).max().unwrap_or(0),
    };
    Ok(Binlog {
        format_version: version,
        succeeded: r.succeeded,
        cancelled: r.cancelled,
        duration_ms,
        projects: r.projects,
        diagnostics: r.diagnostics,
        messages: r.messages,
        truncated: r.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_messages_like_string_format() {
        let args = ["a".to_owned(), "b".to_owned()];
        assert_eq!(format_message("{0} -> {1}", &args), "a -> b");
        assert_eq!(format_message("{1:N0} {{x}} {0,-5}", &args), "b {x} a");
        assert_eq!(format_message("{5}", &args), "{5}");
        assert_eq!(format_message("{0}", &[]), "{0}");
    }

    #[test]
    fn reads_seven_bit_integers() {
        let mut c = Cursor::new(&[0x7f, 0x80, 0x01, 0xfe, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(c.int().unwrap(), 127);
        assert_eq!(c.int().unwrap(), 128);
        assert_eq!(c.int().unwrap(), -2);
    }

    #[test]
    fn rejects_old_and_broken_logs() {
        assert!(parse(&[]).is_err());
        assert!(parse(&9i32.to_le_bytes()).is_err());
    }

    /// Set `BINLOG` to a binary log to read (e.g. from `dotnet build -bl`).
    #[test]
    #[ignore = "needs a binary log"]
    fn reads_a_real_log() {
        let path = std::env::var("BINLOG").expect("BINLOG");
        let log = read(Path::new(&path)).unwrap();
        assert!(log.succeeded, "{:?}", log.diagnostics);
        assert!(!log.projects.is_empty());
        let names: Vec<_> = log.projects.iter().map(|p| p.name.as_str()).collect();
        eprintln!("{} ms, projects {names:?}", log.duration_ms);
        let core = log.projects.iter().find(|p| p.targets.iter().any(|t| t.name == "CoreCompile")).unwrap();
        let compile = core.targets.iter().find(|t| t.name == "CoreCompile").unwrap();
        eprintln!("{} CoreCompile {:?} tasks {:?}", core.name, compile.skipped, compile.tasks.len());
        let json = serde_json::to_string(&log).unwrap();
        eprintln!("json {} bytes, truncated {}", json.len(), log.truncated);
    }
}
