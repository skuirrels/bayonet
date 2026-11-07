//! .NET solutions for the Solution Explorer: the tree of solution folders, projects
//! and solution items that a `.sln`, `.slnx` or `.slnf` file declares.
//!
//! Only the files are read; nothing is evaluated (MSBuild and NuGet belong to the
//! .NET sidecar), so a solution opens instantly and also when no SDK is installed.

pub mod aspire;
pub mod avalonia;
pub mod binlog;
pub mod diagnostics;
pub mod ef;
pub mod generators;
pub mod helper;
pub mod msbuild;
pub mod nuget;
mod project;
pub mod publish;
pub mod secrets;
mod sln;
mod slnx;
pub mod templates;
pub mod testing;
pub mod trace_source;

pub use project::{LaunchProfile, ProjectInfo, launch_profiles, read_project};

use ide_core::{IdeError, IdeResult, IpcPath};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SolutionFormat {
    /// The classic text format.
    Sln,
    /// The XML format (SDK 9.0.200 and later).
    Slnx,
    /// A solution filter: a subset of another solution's projects.
    Slnf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Solution {
    pub path: IpcPath,
    /// File name without the extension, as Visual Studio shows it.
    pub name: String,
    pub format: SolutionFormat,
    /// Top level: folders first, then projects, then files, each by name.
    pub items: Vec<SolutionItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SolutionItem {
    #[serde(rename_all = "camelCase")]
    Folder { name: String, items: Vec<SolutionItem> },
    #[serde(rename_all = "camelCase")]
    Project {
        name: String,
        path: IpcPath,
        language: ProjectLanguage,
        /// The project file is not on disk.
        missing: bool,
    },
    /// A solution item: a file listed in a solution folder.
    #[serde(rename_all = "camelCase")]
    File { path: IpcPath },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProjectLanguage {
    CSharp,
    FSharp,
    VisualBasic,
    Sql,
    Other,
}

impl ProjectLanguage {
    fn of(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("csproj") => Self::CSharp,
            Some("fsproj") => Self::FSharp,
            Some("vbproj") => Self::VisualBasic,
            Some("sqlproj") => Self::Sql,
            _ => Self::Other,
        }
    }
}

impl Solution {
    /// Paths of the projects that exist, in tree order.
    pub fn projects(&self) -> Vec<PathBuf> {
        fn walk(items: &[SolutionItem], out: &mut Vec<PathBuf>) {
            for item in items {
                match item {
                    SolutionItem::Folder { items, .. } => walk(items, out),
                    SolutionItem::Project { path, missing: false, .. } => out.push(path.as_ref().to_path_buf()),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }
}

/// Whether `path` names an MSBuild project file of a language the Solution Explorer knows.
pub fn is_project_file(path: &Path) -> bool {
    ProjectLanguage::of(path) != ProjectLanguage::Other
        || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("proj"))
}

/// The solution formats by extension.
pub fn format_of(path: &Path) -> Option<SolutionFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "sln" => Some(SolutionFormat::Sln),
        "slnx" => Some(SolutionFormat::Slnx),
        "slnf" => Some(SolutionFormat::Slnf),
        _ => None,
    }
}

/// Solution files directly in `dir`: the one chosen last ([`choose_solution`]) first, then
/// `.slnx` before `.sln` before `.slnf`, then by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let mut found = by_format(dir);
    if let Some(i) = chosen(dir).and_then(|c| found.iter().position(|p| *p == c)) {
        let chosen = found.remove(i);
        found.insert(0, chosen);
    }
    found
}

/// Per-user file (git-ignored) naming the solution the user opened, relative to the root.
const CHOSEN_FILE: &str = "solution.local";

fn chosen(dir: &Path) -> Option<PathBuf> {
    let name = std::fs::read_to_string(ide_core::project_dir::path(dir, CHOSEN_FILE)).ok()?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(['/', '\\'])).then(|| dir.join(name))
}

/// Whether `path` is a solution file on disk (`.sln`, `.slnx` or `.slnf`).
pub fn is_solution_file(path: &Path) -> bool {
    format_of(path).is_some() && path.is_file()
}

/// The folder that opens as the project for the solution file `path`; None for any other path.
pub fn solution_folder(path: &Path) -> Option<&Path> {
    is_solution_file(path).then(|| path.parent()).flatten()
}

/// Makes the solution file `path` its folder's solution, which [`find`] then lists first.
pub fn choose_solution(path: &Path) -> IdeResult<()> {
    let (Some(dir), Some(name)) = (solution_folder(path), path.file_name()) else {
        return Err(IdeError::invalid(format!("{} is not a solution file", path.display())));
    };
    remember(dir, &name.to_string_lossy())
}

/// After `old` was converted to `new` (SLN-6): a chosen `old` becomes `new`.
pub fn solution_replaced(old: &Path, new: &Path) -> IdeResult<()> {
    let (Some(dir), Some(name)) = (new.parent(), new.file_name()) else { return Ok(()) };
    if old.parent() == Some(dir) && chosen(dir).as_deref() == Some(old) {
        remember(dir, &name.to_string_lossy())?;
    }
    Ok(())
}

fn remember(dir: &Path, name: &str) -> IdeResult<()> {
    ide_core::project_dir::ignore(dir, &[&format!("/{CHOSEN_FILE}")])?;
    let file = ide_core::project_dir::path(dir, CHOSEN_FILE);
    if std::fs::read_to_string(&file).is_ok_and(|current| current.trim() == name) {
        return Ok(());
    }
    std::fs::write(file, format!("{name}\n"))?;
    Ok(())
}

fn by_format(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(SolutionFormat, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| format_of(&p).map(|f| (f, p)))
        .collect();
    let rank = |f: SolutionFormat| match f {
        SolutionFormat::Slnx => 0,
        SolutionFormat::Sln => 1,
        SolutionFormat::Slnf => 2,
    };
    found.sort_by(|(fa, a), (fb, b)| rank(*fa).cmp(&rank(*fb)).then_with(|| a.cmp(b)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Reads the solution at `path`.
pub fn read(path: &Path) -> IdeResult<Solution> {
    let format = format_of(path).ok_or_else(|| IdeError::invalid(format!("{} is not a solution", path.display())))?;
    let text = std::fs::read_to_string(path)?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut items = match format {
        SolutionFormat::Sln => sln::parse(&text, dir)?,
        SolutionFormat::Slnx => slnx::parse(&text, dir)?,
        SolutionFormat::Slnf => return read_filter(path, &text),
    };
    sort(&mut items);
    Ok(Solution { path: path.into(), name: stem(path), format, items })
}

/// A `.slnf`: `{ "solution": { "path": "App.sln", "projects": ["src\\App\\App.csproj"] } }`,
/// paths relative to the filter's directory and to the solution's, respectively.
fn read_filter(path: &Path, text: &str) -> IdeResult<Solution> {
    #[derive(Deserialize)]
    struct Filter {
        solution: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        path: String,
        #[serde(default)]
        projects: Vec<String>,
    }
    let filter: Filter = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| IdeError::invalid(format!("{}: {e}", path.display())))?;
    let base_path = resolve(path.parent().unwrap_or(Path::new("")), &filter.solution.path);
    if format_of(&base_path).is_none_or(|f| f == SolutionFormat::Slnf) {
        return Err(IdeError::invalid(format!("{}: {} is not a solution", path.display(), filter.solution.path)));
    }
    let base = read(&base_path)?;
    let base_dir = base_path.parent().unwrap_or(Path::new(""));
    let keep: Vec<PathBuf> = filter.solution.projects.iter().map(|p| resolve(base_dir, p)).collect();
    let items = retain_projects(base.items, &keep);
    Ok(Solution { path: path.into(), name: stem(path), format: SolutionFormat::Slnf, items })
}

/// Drops projects not in `keep`, and the folders that end up empty of projects.
fn retain_projects(items: Vec<SolutionItem>, keep: &[PathBuf]) -> Vec<SolutionItem> {
    items
        .into_iter()
        .filter_map(|item| match item {
            SolutionItem::Project { ref path, .. } => keep.iter().any(|k| k == path.as_ref() as &Path).then_some(item),
            SolutionItem::Folder { name, items } => {
                let items = retain_projects(items, keep);
                has_project(&items).then_some(SolutionItem::Folder { name, items })
            }
            SolutionItem::File { .. } => Some(item),
        })
        .collect()
}

fn has_project(items: &[SolutionItem]) -> bool {
    items.iter().any(|i| match i {
        SolutionItem::Project { .. } => true,
        SolutionItem::Folder { items, .. } => has_project(items),
        SolutionItem::File { .. } => false,
    })
}

fn sort(items: &mut [SolutionItem]) {
    fn key(item: &SolutionItem) -> (u8, String) {
        match item {
            SolutionItem::Folder { name, .. } => (0, name.to_lowercase()),
            SolutionItem::Project { name, .. } => (1, name.to_lowercase()),
            SolutionItem::File { path } => {
                (2, path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default())
            }
        }
    }
    items.sort_by_key(key);
    for item in items {
        if let SolutionItem::Folder { items, .. } = item {
            sort(items);
        }
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A project or item path as written in a solution (`src\App\App.csproj`), made
/// absolute against `dir` without touching the file system.
pub(crate) fn resolve(dir: &Path, written: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in dir.join(written.replace('\\', "/")).components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn project(path: PathBuf) -> SolutionItem {
    SolutionItem::Project {
        name: stem(&path),
        language: ProjectLanguage::of(&path),
        missing: !path.is_file(),
        path: path.into(),
    }
}

#[cfg(test)]
mod tests;
