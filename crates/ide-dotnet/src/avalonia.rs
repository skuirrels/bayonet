//! Avalonia previewer (CI-16): renders an `.axaml` file as the user types, with
//! Avalonia's own designer host (`Avalonia.Designer.HostApp`, shipped in the Avalonia
//! package). The project is built once; the host loads its assembly in design mode and
//! connects back to a loopback TCP port. Messages are framed as a little-endian length,
//! the message type's GUID and a BSON body (`Avalonia.Remote.Protocol`). The IDE sends
//! the XAML text on every change; the host answers with the result and RGBA frames,
//! which are passed on as PNG images.

use ide_core::{CancellationToken, IdeError, IdeResult};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// What the preview reports to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewEvent {
    Phase {
        phase: PreviewPhase,
        message: Option<String>,
    },
    /// A rendered frame: a PNG (base64), its size in device pixels and the scale it was drawn at.
    #[serde(rename_all = "camelCase")]
    Frame {
        png: String,
        width: u32,
        height: u32,
        scale: f64,
    },
    /// The XAML could not be loaded; the last good frame stays. 0-based line and column.
    XamlError {
        message: String,
        line: Option<u32>,
        column: Option<u32>,
    },
    /// The XAML loaded.
    XamlLoaded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PreviewPhase {
    Building,
    Starting,
    Ready,
    Failed,
    Stopped,
}

/// What the designer host needs: the built app and the host matching its Avalonia version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewTarget {
    pub project: PathBuf,
    pub host_app: PathBuf,
    pub runtime_config: PathBuf,
    pub deps_file: PathBuf,
    pub assembly: PathBuf,
}

/// The project a XAML file belongs to: the nearest folder above it with one project file.
pub fn project_of(xaml: &Path) -> Option<PathBuf> {
    let mut dir = xaml.parent();
    while let Some(d) = dir {
        let projects: Vec<PathBuf> = std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "csproj" || e == "fsproj" || e == "vbproj"))
            .collect();
        if let [one] = projects.as_slice() {
            return Some(one.clone());
        }
        dir = d.parent();
    }
    None
}

/// `/Views/MainWindow.axaml`: the file's path in the project, which resolves relative resource URIs.
pub fn project_relative(project: &Path, xaml: &Path) -> String {
    let dir = project.parent().unwrap_or(Path::new(""));
    let rel = xaml.strip_prefix(dir).unwrap_or(xaml);
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    format!("/{}", parts.join("/"))
}

const PROPERTIES: [&str; 4] =
    ["AvaloniaPreviewerNetCoreToolPath", "ProjectRuntimeConfigFilePath", "ProjectDepsFilePath", "TargetPath"];

/// The target from `dotnet msbuild -getProperty:…` output.
pub fn target_from(project: &Path, properties: &[(String, String)]) -> IdeResult<PreviewTarget> {
    let get = |name: &str| {
        properties.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim()).filter(|v| !v.is_empty()).map(PathBuf::from)
    };
    let host_app = get(PROPERTIES[0])
        .ok_or_else(|| IdeError::unavailable("the project does not reference Avalonia (no previewer host)"))?;
    let missing = |what: &str| IdeError::unavailable(format!("MSBuild did not report the project's {what}"));
    Ok(PreviewTarget {
        project: project.to_path_buf(),
        host_app,
        runtime_config: get(PROPERTIES[1]).ok_or_else(|| missing("runtime config"))?,
        deps_file: get(PROPERTIES[2]).ok_or_else(|| missing("deps file"))?,
        assembly: get(PROPERTIES[3]).ok_or_else(|| missing("output assembly"))?,
    })
}

/// `dotnet` without telemetry or the first-run banner, its messages in English (they are parsed).
fn sdk(dotnet: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(dotnet);
    cmd.env("DOTNET_CLI_TELEMETRY_OPTOUT", "1").env("DOTNET_NOLOGO", "1").env("DOTNET_CLI_UI_LANGUAGE", "en");
    cmd
}

/// Builds `project` and asks MSBuild for what the previewer needs.
pub async fn prepare(dotnet: &Path, project: &Path, cancel: &CancellationToken) -> IdeResult<PreviewTarget> {
    let build = sdk(dotnet)
        .args(["build", "-nologo", "-v:q", "-clp:NoSummary"])
        .arg(project)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::select! {
        _ = cancel.cancelled() => return Err(IdeError::cancelled()),
        out = build => out?,
    };
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout);
        let errors = crate::generators::errors(&text);
        let detail =
            if errors.is_empty() { text.trim().lines().last().unwrap_or("").to_owned() } else { errors.join("\n") };
        return Err(IdeError::invalid(format!("the project did not build\n{detail}")));
    }
    let mut cmd = sdk(dotnet);
    cmd.arg("msbuild").arg(project).arg("-nologo");
    for p in PROPERTIES {
        cmd.arg(format!("-getProperty:{p}"));
    }
    let out = cmd.stdin(Stdio::null()).kill_on_drop(true).output().await?;
    target_from(project, &crate::generators::parse_properties(&String::from_utf8_lossy(&out.stdout)))
}

// --- BSON (the subset Avalonia.Remote.Protocol uses) --------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Bson {
    Double(f64),
    String(String),
    Document(Vec<(String, Bson)>),
    Array(Vec<Bson>),
    Binary(Vec<u8>),
    Bool(bool),
    Null,
    Int32(i32),
    Int64(i64),
}

impl Bson {
    pub fn get(&self, key: &str) -> Option<&Bson> {
        match self {
            Bson::Document(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match *self {
            Bson::Double(v) => Some(v),
            Bson::Int32(v) => Some(v.into()),
            Bson::Int64(v) => Some(v as f64),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match *self {
            Bson::Int32(v) => Some(v.into()),
            Bson::Int64(v) => Some(v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Bson::String(s) => Some(s),
            _ => None,
        }
    }
}

pub fn encode_document(fields: &[(&str, Bson)]) -> Vec<u8> {
    let mut out = vec![0; 4];
    for (k, v) in fields {
        encode_element(&mut out, k, v);
    }
    out.push(0);
    let len = out.len() as i32;
    out[..4].copy_from_slice(&len.to_le_bytes());
    out
}

fn encode_element(out: &mut Vec<u8>, key: &str, v: &Bson) {
    let tag = match v {
        Bson::Double(_) => 0x01,
        Bson::String(_) => 0x02,
        Bson::Document(_) => 0x03,
        Bson::Array(_) => 0x04,
        Bson::Binary(_) => 0x05,
        Bson::Bool(_) => 0x08,
        Bson::Null => 0x0A,
        Bson::Int32(_) => 0x10,
        Bson::Int64(_) => 0x12,
    };
    out.push(tag);
    out.extend_from_slice(key.as_bytes());
    out.push(0);
    match v {
        Bson::Double(d) => out.extend_from_slice(&d.to_le_bytes()),
        Bson::String(s) => {
            out.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        Bson::Document(fields) => {
            let refs: Vec<(&str, Bson)> = fields.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            out.extend(encode_document(&refs));
        }
        Bson::Array(items) => {
            let keys: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
            let refs: Vec<(&str, Bson)> = keys.iter().map(String::as_str).zip(items.iter().cloned()).collect();
            out.extend(encode_document(&refs));
        }
        Bson::Binary(b) => {
            out.extend_from_slice(&(b.len() as i32).to_le_bytes());
            out.push(0);
            out.extend_from_slice(b);
        }
        Bson::Bool(b) => out.push(u8::from(*b)),
        Bson::Null => {}
        Bson::Int32(i) => out.extend_from_slice(&i.to_le_bytes()),
        Bson::Int64(i) => out.extend_from_slice(&i.to_le_bytes()),
    }
}

/// Decodes one document (the whole of `bytes`).
pub fn decode_document(bytes: &[u8]) -> IdeResult<Bson> {
    let mut at = 0;
    let doc = decode_doc(bytes, &mut at)?;
    Ok(Bson::Document(doc))
}

fn bad() -> IdeError {
    IdeError::invalid("malformed message from the Avalonia previewer")
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> IdeResult<&'a [u8]> {
    let s = bytes.get(*at..*at + n).ok_or_else(bad)?;
    *at += n;
    Ok(s)
}

fn i32_at(bytes: &[u8], at: &mut usize) -> IdeResult<i32> {
    Ok(i32::from_le_bytes(take(bytes, at, 4)?.try_into().map_err(|_| bad())?))
}

fn decode_doc(bytes: &[u8], at: &mut usize) -> IdeResult<Vec<(String, Bson)>> {
    let start = *at;
    let len = usize::try_from(i32_at(bytes, at)?).map_err(|_| bad())?;
    let end = start.checked_add(len).filter(|&e| e <= bytes.len()).ok_or_else(bad)?;
    let mut fields = Vec::new();
    while *at < end - 1 {
        let tag = take(bytes, at, 1)?[0];
        let nul = bytes[*at..end].iter().position(|&b| b == 0).ok_or_else(bad)?;
        let key = String::from_utf8_lossy(&bytes[*at..*at + nul]).into_owned();
        *at += nul + 1;
        let value = match tag {
            0x01 => Bson::Double(f64::from_le_bytes(take(bytes, at, 8)?.try_into().map_err(|_| bad())?)),
            0x02 => {
                let n = usize::try_from(i32_at(bytes, at)?).map_err(|_| bad())?;
                let s = take(bytes, at, n)?;
                Bson::String(String::from_utf8_lossy(&s[..n.saturating_sub(1)]).into_owned())
            }
            0x03 => Bson::Document(decode_doc(bytes, at)?),
            0x04 => Bson::Array(decode_doc(bytes, at)?.into_iter().map(|(_, v)| v).collect()),
            0x05 => {
                let n = usize::try_from(i32_at(bytes, at)?).map_err(|_| bad())?;
                take(bytes, at, 1)?;
                Bson::Binary(take(bytes, at, n)?.to_vec())
            }
            0x08 => Bson::Bool(take(bytes, at, 1)?[0] != 0),
            0x0A => Bson::Null,
            0x10 => Bson::Int32(i32_at(bytes, at)?),
            0x12 => Bson::Int64(i64::from_le_bytes(take(bytes, at, 8)?.try_into().map_err(|_| bad())?)),
            _ => return Err(bad()),
        };
        fields.push((key, value));
    }
    *at = end;
    Ok(fields)
}

// --- protocol ------------------------------------------------------------------------------

/// `Guid.ToByteArray()` of a GUID string: the first three groups little-endian.
pub fn guid_bytes(guid: &str) -> [u8; 16] {
    let hex: String = guid.chars().filter(char::is_ascii_hexdigit).collect();
    let mut b = [0u8; 16];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    b[0..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    b
}

pub mod messages {
    pub const START_DESIGNER_SESSION: &str = "854887CF-2694-4EB6-B499-7461B6FB96C7";
    pub const UPDATE_XAML: &str = "9AEC9A2E-6315-4066-B4BA-E9A9EFD0F8CC";
    pub const UPDATE_XAML_RESULT: &str = "B7A70093-0C5D-47FD-9261-22086D43A2E2";
    pub const CLIENT_SUPPORTED_PIXEL_FORMATS: &str = "63481025-7016-43FE-BADC-F2FD0F88609E";
    pub const CLIENT_RENDER_INFO: &str = "7A3c25d3-3652-438D-8EF1-86E942CC96C0";
    pub const CLIENT_VIEWPORT_ALLOCATED: &str = "BD7A8DE6-3DB8-4A13-8583-D6D4AB189A31";
    pub const REQUEST_VIEWPORT_RESIZE: &str = "9B47B3D8-61DF-4C38-ACD4-8C1BB72554AC";
    pub const FRAME: &str = "F58313EE-FE69-4536-819D-F52EDF201A0E";
    pub const FRAME_RECEIVED: &str = "68014F8A-289D-4851-8D34-5367EDA7F827";
}

/// `PixelFormat.Rgba8888`.
const RGBA8888: i32 = 1;

/// One framed message: length, GUID, BSON body.
pub fn frame(guid: &str, body: &[(&str, Bson)]) -> Vec<u8> {
    let doc = encode_document(body);
    let mut out = Vec::with_capacity(doc.len() + 20);
    out.extend_from_slice(&(doc.len() as i32).to_le_bytes());
    out.extend_from_slice(&guid_bytes(guid));
    out.extend(doc);
    out
}

async fn read_message(stream: &mut TcpStream) -> IdeResult<([u8; 16], Bson)> {
    let mut head = [0u8; 20];
    stream.read_exact(&mut head).await?;
    let len = usize::try_from(i32::from_le_bytes(head[..4].try_into().map_err(|_| bad())?)).map_err(|_| bad())?;
    let mut guid = [0u8; 16];
    guid.copy_from_slice(&head[4..]);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok((guid, decode_document(&body)?))
}

/// RGBA rows (`stride` bytes apart) as a PNG.
pub fn png_of(data: &[u8], width: u32, height: u32, stride: usize) -> IdeResult<Vec<u8>> {
    let row = width as usize * 4;
    if stride < row || data.len() < stride * (height as usize).saturating_sub(1) + row {
        return Err(bad());
    }
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut w = enc.write_header().map_err(|e| IdeError::internal(e.to_string()))?;
    w.write_image_data(&pixels).map_err(|e| IdeError::internal(e.to_string()))?;
    w.finish().map_err(|e| IdeError::internal(e.to_string()))?;
    Ok(out)
}

/// The UI event for an `UpdateXamlResultMessage`.
pub fn xaml_result(body: &Bson) -> PreviewEvent {
    let Some(error) = body.get("Error").and_then(Bson::as_str).filter(|e| !e.is_empty()) else {
        return PreviewEvent::XamlLoaded;
    };
    let exception = body.get("Exception");
    let detail = exception.and_then(|e| e.get("Message")).and_then(Bson::as_str);
    let number =
        |k: &str| exception.and_then(|e| e.get(k)).and_then(Bson::as_i64).and_then(|n| u32::try_from(n - 1).ok());
    let raw = detail.unwrap_or_else(|| error.lines().next().unwrap_or(error));
    let (message, at) = tidy_error(raw);
    PreviewEvent::XamlError {
        message,
        line: number("LineNumber").or(at.map(|a| a.0)),
        column: number("LinePosition").or(at.map(|a| a.1)),
    }
}

/// The compiler's errors come as an `AggregateException` ("One or more errors occurred.
/// (a) (a) (b)") whose parts end in "Line N, position M.": the distinct parts without
/// their positions, and the first (0-based) position.
pub fn tidy_error(message: &str) -> (String, Option<(u32, u32)>) {
    let parts: Vec<&str> = match message.strip_prefix("One or more errors occurred.") {
        Some(rest) => {
            let mut parts = Vec::new();
            let (mut depth, mut start) = (0usize, None);
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => {
                        if depth == 0 {
                            start = Some(i + 1);
                        }
                        depth += 1;
                    }
                    ')' if depth > 0 => {
                        depth -= 1;
                        if depth == 0
                            && let Some(st) = start.take()
                        {
                            parts.push(rest[st..i].trim());
                        }
                    }
                    _ => {}
                }
            }
            if parts.is_empty() { vec![message] } else { parts }
        }
        None => vec![message],
    };
    let mut at = None;
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let (text, pos) = split_position(part);
        at = at.or(pos);
        if !out.iter().any(|o| o == text) {
            out.push(text.to_owned());
        }
    }
    (out.join("\n"), at)
}

/// "… Line 16, position 6." → ("…", Some((15, 5))).
fn split_position(text: &str) -> (&str, Option<(u32, u32)>) {
    let Some(i) = text.rfind(" Line ") else { return (text, None) };
    let tail = text[i + 6..].trim_end_matches('.');
    let mut nums = tail.split(", position ");
    let line = nums.next().and_then(|n| n.trim().parse::<u32>().ok());
    let col = nums.next().and_then(|n| n.trim().parse::<u32>().ok());
    match (line, col) {
        (Some(l), Some(c)) if l > 0 && c > 0 => (text[..i].trim_end(), Some((l - 1, c - 1))),
        _ => (text, None),
    }
}

// --- sessions ------------------------------------------------------------------------------

/// XAML updates for running previews, by request id.
static SESSIONS: LazyLock<Mutex<HashMap<u64, mpsc::UnboundedSender<String>>>> = LazyLock::new(Default::default);

/// Sends new XAML text to the preview started as `id`; false when it is not running.
pub fn update(id: u64, xaml: String) -> bool {
    SESSIONS.lock().ok().and_then(|s| s.get(&id).map(|tx| tx.send(xaml).is_ok())).unwrap_or(false)
}

struct Registered(u64);

impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut s) = SESSIONS.lock() {
            s.remove(&self.0);
        }
    }
}

/// How long the designer host may take to connect.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Builds the project of `xaml`, starts the designer host and keeps the preview
/// current with the text sent through [`update`] for `id`, until `cancel`.
pub async fn run(
    id: u64,
    dotnet: &Path,
    xaml: &Path,
    text: String,
    scale: f64,
    cancel: &CancellationToken,
    emit: &(dyn Fn(PreviewEvent) + Send + Sync),
) -> IdeResult<()> {
    let (tx, mut updates) = mpsc::unbounded_channel();
    SESSIONS.lock().map_err(|_| IdeError::internal("preview registry poisoned"))?.insert(id, tx);
    let _registered = Registered(id);
    let project = project_of(xaml).ok_or_else(|| IdeError::not_found("no project file above the XAML file"))?;
    emit(PreviewEvent::Phase { phase: PreviewPhase::Building, message: None });
    let target = prepare(dotnet, &project, cancel).await?;
    emit(PreviewEvent::Phase { phase: PreviewPhase::Starting, message: None });

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let mut host = sdk(dotnet)
        .arg("exec")
        .arg("--runtimeconfig")
        .arg(&target.runtime_config)
        .arg("--depsfile")
        .arg(&target.deps_file)
        .arg(&target.host_app)
        .args(["--method", "avalonia-remote", "--transport"])
        .arg(format!("tcp-bson://127.0.0.1:{port}/"))
        .args(["--session-id", &format!("openidea-{id}")])
        .arg(&target.assembly)
        .current_dir(target.project.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    // The host logs to stderr; the last lines explain an early exit.
    let log = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    if let Some(stderr) = host.stderr.take() {
        let log = log.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "ide_dotnet::avalonia", "{line}");
                if let Ok(mut l) = log.lock() {
                    if l.len() >= 20 {
                        l.remove(0);
                    }
                    l.push(line);
                }
            }
        });
    }
    let exited = |log: &Mutex<Vec<String>>| {
        let tail = log.lock().map(|l| l.join("\n")).unwrap_or_default();
        IdeError::unavailable(format!("the Avalonia previewer stopped\n{tail}").trim_end().to_owned())
    };
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(IdeError::cancelled()),
        _ = host.wait() => return Err(exited(&log)),
        accepted = tokio::time::timeout(CONNECT_TIMEOUT, listener.accept()) => match accepted {
            Ok(Ok((s, _))) => s,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(IdeError::unavailable("the Avalonia previewer did not connect")),
        },
    };
    drop(listener);
    let _ = stream.set_nodelay(true);
    let dpi = 96.0 * scale;
    let path_in_project = project_relative(&project, xaml);
    let update_xaml = |xaml: &str| {
        frame(
            messages::UPDATE_XAML,
            &[
                ("Xaml", Bson::String(xaml.to_owned())),
                ("AssemblyPath", Bson::String(target.assembly.display().to_string())),
                ("XamlFileProjectPath", Bson::String(path_in_project.clone())),
            ],
        )
    };
    let mut started = false;
    let mut latest = text;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = host.wait() => return Err(exited(&log)),
            next = updates.recv() => {
                let Some(next) = next else { break };
                latest = next;
                if started {
                    stream.write_all(&update_xaml(&latest)).await?;
                }
            }
            message = read_message(&mut stream) => {
                let (guid, body) = match message {
                    Ok(m) => m,
                    Err(_) => return Err(exited(&log)),
                };
                if guid == guid_bytes(messages::START_DESIGNER_SESSION) {
                    let mut out = frame(messages::CLIENT_SUPPORTED_PIXEL_FORMATS, &[("Formats", Bson::Array(vec![Bson::Int32(RGBA8888)]))]);
                    out.extend(frame(messages::CLIENT_RENDER_INFO, &[("DpiX", Bson::Double(dpi)), ("DpiY", Bson::Double(dpi))]));
                    out.extend(update_xaml(&latest));
                    stream.write_all(&out).await?;
                    started = true;
                    emit(PreviewEvent::Phase { phase: PreviewPhase::Ready, message: None });
                } else if guid == guid_bytes(messages::UPDATE_XAML_RESULT) {
                    emit(xaml_result(&body));
                } else if guid == guid_bytes(messages::REQUEST_VIEWPORT_RESIZE) {
                    let w = body.get("Width").and_then(Bson::as_f64).unwrap_or(0.0);
                    let h = body.get("Height").and_then(Bson::as_f64).unwrap_or(0.0);
                    let reply = frame(
                        messages::CLIENT_VIEWPORT_ALLOCATED,
                        &[("Width", Bson::Double(w)), ("Height", Bson::Double(h)), ("DpiX", Bson::Double(dpi)), ("DpiY", Bson::Double(dpi))],
                    );
                    stream.write_all(&reply).await?;
                } else if guid == guid_bytes(messages::FRAME) {
                    let seq = body.get("SequenceId").and_then(Bson::as_i64).unwrap_or(0);
                    let width = body.get("Width").and_then(Bson::as_i64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
                    let height = body.get("Height").and_then(Bson::as_i64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
                    let stride = body.get("Stride").and_then(Bson::as_i64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
                    let format = body.get("Format").and_then(Bson::as_i64).unwrap_or(-1);
                    if let (Some(Bson::Binary(data)), true) = (body.get("Data"), format == i64::from(RGBA8888) && width > 0 && height > 0) {
                        let data = data.clone();
                        let png = tokio::task::spawn_blocking(move || png_of(&data, width, height, stride))
                            .await
                            .map_err(|e| IdeError::internal(e.to_string()))??;
                        use base64::Engine as _;
                        let png = base64::engine::general_purpose::STANDARD.encode(png);
                        emit(PreviewEvent::Frame { png, width, height, scale });
                    }
                    stream.write_all(&frame(messages::FRAME_RECEIVED, &[("SequenceId", Bson::Int64(seq))])).await?;
                }
            }
        }
    }
    let _ = host.kill().await;
    emit(PreviewEvent::Phase { phase: PreviewPhase::Stopped, message: None });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_round_trips_the_message_shapes() {
        let doc = encode_document(&[
            ("Xaml", Bson::String("<Window/>".into())),
            ("Formats", Bson::Array(vec![Bson::Int32(1)])),
            ("DpiX", Bson::Double(192.0)),
            ("SequenceId", Bson::Int64(7)),
            ("Data", Bson::Binary(vec![1, 2, 3])),
            ("Exception", Bson::Document(vec![("LineNumber".into(), Bson::Int32(3))])),
            ("Error", Bson::Null),
        ]);
        let back = decode_document(&doc).unwrap();
        assert_eq!(back.get("Xaml"), Some(&Bson::String("<Window/>".into())));
        assert_eq!(back.get("Formats"), Some(&Bson::Array(vec![Bson::Int32(1)])));
        assert_eq!(back.get("DpiX"), Some(&Bson::Double(192.0)));
        assert_eq!(back.get("SequenceId"), Some(&Bson::Int64(7)));
        assert_eq!(back.get("Data"), Some(&Bson::Binary(vec![1, 2, 3])));
        assert_eq!(back.get("Exception").and_then(|e| e.get("LineNumber")), Some(&Bson::Int32(3)));
        assert_eq!(back.get("Error"), Some(&Bson::Null));
        assert!(decode_document(&doc[..doc.len() - 3]).is_err());
    }

    #[test]
    fn a_bson_document_matches_the_spec_bytes() {
        // {"a": 1} from bsonspec.org's examples.
        assert_eq!(encode_document(&[("a", Bson::Int32(1))]), [12, 0, 0, 0, 0x10, b'a', 0, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn guids_use_dotnets_byte_order() {
        // Guid.Parse("00112233-4455-6677-8899-aabbccddeeff").ToByteArray()
        assert_eq!(
            guid_bytes("00112233-4455-6677-8899-AABBCCDDEEFF"),
            [0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]
        );
        let framed = frame(messages::FRAME_RECEIVED, &[("SequenceId", Bson::Int64(1))]);
        assert_eq!(i32::from_le_bytes(framed[..4].try_into().unwrap()) as usize, framed.len() - 20);
    }

    #[test]
    fn xaml_errors_carry_a_zero_based_position() {
        let body = Bson::Document(vec![
            ("Error".into(), Bson::String("Avalonia.Markup.Xaml.XamlLoadException: bad\n   at …".into())),
            (
                "Exception".into(),
                Bson::Document(vec![
                    ("Message".into(), Bson::String("Unable to resolve type Buton".into())),
                    ("LineNumber".into(), Bson::Int32(4)),
                    ("LinePosition".into(), Bson::Int32(6)),
                ]),
            ),
        ]);
        assert_eq!(
            xaml_result(&body),
            PreviewEvent::XamlError { message: "Unable to resolve type Buton".into(), line: Some(3), column: Some(5) }
        );
        assert_eq!(xaml_result(&Bson::Document(vec![("Error".into(), Bson::Null)])), PreviewEvent::XamlLoaded);
    }

    #[test]
    fn aggregate_xaml_errors_are_tidied() {
        let raw = "One or more errors occurred. (Unable to resolve type Buton from namespace https://github.com/avaloniaui Line 16, position 6.) (Unable to resolve type Buton from namespace https://github.com/avaloniaui Line 16, position 6.) (Unable to resolve suitable regular or attached property Content on type :.{Unknown type} Line 16, position 6.)";
        let (message, at) = tidy_error(raw);
        assert_eq!(
            message,
            "Unable to resolve type Buton from namespace https://github.com/avaloniaui\nUnable to resolve suitable regular or attached property Content on type :.{Unknown type}"
        );
        assert_eq!(at, Some((15, 5)));
        assert_eq!(tidy_error("plain"), ("plain".into(), None));
    }

    #[test]
    fn frames_become_pngs_without_row_padding() {
        // 2×1 pixels in rows padded to 12 bytes.
        let data = [255, 0, 0, 255, 0, 255, 0, 255, 9, 9, 9, 9];
        let png = png_of(&data, 2, 1, 12).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(png_of(&data, 4, 1, 12).is_err());
    }

    #[test]
    fn finds_the_project_and_the_path_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("Views");
        std::fs::create_dir_all(&views).unwrap();
        std::fs::write(dir.path().join("App.csproj"), "<Project/>").unwrap();
        let xaml = views.join("MainWindow.axaml");
        let project = project_of(&xaml).unwrap();
        assert_eq!(project, dir.path().join("App.csproj"));
        assert_eq!(project_relative(&project, &xaml), "/Views/MainWindow.axaml");
    }

    #[test]
    fn a_project_without_avalonia_has_no_previewer() {
        let props = vec![("TargetPath".to_owned(), "/p/bin/App.dll".to_owned())];
        assert!(target_from(Path::new("/p/App.csproj"), &props).unwrap_err().message.contains("Avalonia"));
    }
}
