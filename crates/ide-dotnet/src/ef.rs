//! Entity Framework Core (PLT-2): `dotnet ef` migrations, database updates and
//! DbContext scaffolding. The IDE runs the commands in the Run tool window; this
//! module finds the tool, the startup project and the existing migrations, and
//! builds the command lines.

use crate::read_project;
use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// The package that makes a project an EF Core project.
pub const EF_CORE_PACKAGE: &str = "Microsoft.EntityFrameworkCore";
/// The package `dotnet ef` needs in the startup project.
pub const EF_DESIGN_PACKAGE: &str = "Microsoft.EntityFrameworkCore.Design";

/// What to do with `dotnet ef`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EfOperation {
    #[serde(rename_all = "camelCase")]
    AddMigration {
        name: String,
        output_dir: Option<String>,
    },
    RemoveMigration,
    /// To `migration` (`0` reverts every migration), or to the latest.
    #[serde(rename_all = "camelCase")]
    UpdateDatabase {
        migration: Option<String>,
    },
    /// An idempotent SQL script of every migration.
    #[serde(rename_all = "camelCase")]
    Script {
        output: String,
    },
    #[serde(rename_all = "camelCase")]
    ScaffoldDbContext {
        connection: String,
        provider: String,
        output_dir: Option<String>,
        context: Option<String>,
    },
}

/// A command that installs a missing tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolInstall {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<IpcPath>,
}

/// The command line of an [`EfOperation`], or why it cannot run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum EfCommand {
    #[serde(rename_all = "camelCase")]
    Ready { command: String, args: Vec<String>, cwd: IpcPath, startup_project: IpcPath },
    #[serde(rename_all = "camelCase")]
    MissingTool { tool: String, message: String, install: Option<ToolInstall> },
}

/// Whether the project in `project_dir` uses EF Core, from its restore output.
pub fn uses_ef_core(project_dir: &Path) -> bool {
    std::fs::read_to_string(project_dir.join("obj").join("project.assets.json")).is_ok_and(|text| {
        let text = text.to_ascii_lowercase();
        text.contains(&format!("\"{}/", EF_CORE_PACKAGE.to_ascii_lowercase()))
    })
}

/// The executable that runs migrations of `project`: itself when it can start,
/// else the first executable project (of `candidates`) that references it.
pub fn startup_project(project: &Path, candidates: &[PathBuf]) -> PathBuf {
    if read_project(project).is_ok_and(|p| p.is_executable()) {
        return project.to_path_buf();
    }
    let canonical = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    candidates
        .iter()
        .find(|c| {
            read_project(c).is_ok_and(|p| p.is_executable())
                && crate::templates::project_references(c).is_ok_and(|refs| refs.contains(&canonical))
        })
        .cloned()
        .unwrap_or_else(|| project.to_path_buf())
}

/// `20240101120000_InitialCreate.cs` → `InitialCreate` (designer files and snapshots are not migrations).
fn migration_name(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".cs")?;
    if stem.ends_with(".Designer") {
        return None;
    }
    let (stamp, name) = stem.split_once('_')?;
    (stamp.len() == 14 && stamp.bytes().all(|b| b.is_ascii_digit()) && !name.is_empty()).then_some(name)
}

/// The migrations of the project in `project_dir`, oldest first, from their file names
/// (no build needed). `bin` and `obj` are skipped.
pub fn migrations(project_dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if path.is_dir() {
                if depth < 6 && !name.starts_with('.') && name != "bin" && name != "obj" {
                    walk(&path, depth + 1, out);
                }
            } else if let Some(m) = migration_name(&name) {
                out.push((name.clone(), m.to_owned()));
            }
        }
    }
    let mut found = Vec::new();
    walk(project_dir, 0, &mut found);
    // The timestamp prefix orders them.
    found.sort();
    found.into_iter().map(|(_, m)| m).collect()
}

/// `dotnet ef` arguments (after `ef`) for `op` on `project` with `startup`.
pub fn ef_args(op: &EfOperation, project: &Path, startup: &Path) -> Vec<String> {
    let mut args: Vec<String> = match op {
        EfOperation::AddMigration { name, output_dir } => {
            let mut a = vec!["migrations".into(), "add".into(), name.clone()];
            if let Some(dir) = output_dir.as_deref().filter(|d| !d.is_empty()) {
                a.extend(["--output-dir".into(), dir.to_owned()]);
            }
            a
        }
        EfOperation::RemoveMigration => vec!["migrations".into(), "remove".into()],
        EfOperation::UpdateDatabase { migration } => {
            let mut a = vec!["database".into(), "update".into()];
            a.extend(migration.iter().filter(|m| !m.is_empty()).cloned());
            a
        }
        EfOperation::Script { output } => {
            vec!["migrations".into(), "script".into(), "--idempotent".into(), "--output".into(), output.clone()]
        }
        EfOperation::ScaffoldDbContext { connection, provider, output_dir, context } => {
            let mut a = vec!["dbcontext".into(), "scaffold".into(), connection.clone(), provider.clone()];
            if let Some(dir) = output_dir.as_deref().filter(|d| !d.is_empty()) {
                a.extend(["--output-dir".into(), dir.to_owned()]);
            }
            if let Some(c) = context.as_deref().filter(|c| !c.is_empty()) {
                a.extend(["--context".into(), c.to_owned()]);
            }
            a
        }
    };
    args.extend([
        "--project".into(),
        project.to_string_lossy().into_owned(),
        "--startup-project".into(),
        startup.to_string_lossy().into_owned(),
    ]);
    args
}

/// The global tool's executable (`~/.dotnet/tools/dotnet-ef`).
fn global_tool() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let exe = if cfg!(windows) { "dotnet-ef.exe" } else { "dotnet-ef" };
    let path = PathBuf::from(home).join(".dotnet").join("tools").join(exe);
    path.is_file().then_some(path)
}

/// How to run `dotnet ef` in `dir`: `dotnet ef` when it works there (a restored local tool
/// or the global one), else the global tool's executable (a local tool manifest that is not
/// restored hides it from `dotnet ef`). `None` when neither is installed.
pub fn resolve_tool(dotnet: &Path, dir: &Path) -> Option<(String, Vec<String>)> {
    let works = ide_core::process::prepare(&mut std::process::Command::new(dotnet))
        .args(["ef", "--version"])
        .current_dir(dir)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .output()
        .is_ok_and(|o| o.status.success());
    if works {
        return Some((dotnet.to_string_lossy().into_owned(), vec!["ef".into()]));
    }
    global_tool().map(|p| (p.to_string_lossy().into_owned(), Vec::new()))
}

/// The command for `op` on `project` (`candidates`: the solution's projects, for the startup project).
pub fn command(dotnet: &Path, op: &EfOperation, project: &Path, candidates: &[PathBuf]) -> EfCommand {
    let dir = project.parent().unwrap_or(project);
    let Some((command, prefix)) = resolve_tool(dotnet, dir) else {
        return EfCommand::MissingTool {
            tool: "dotnet-ef".into(),
            message: "Entity Framework Core needs the dotnet-ef tool.".into(),
            install: Some(ToolInstall {
                command: dotnet.to_string_lossy().into_owned(),
                args: vec!["tool".into(), "install".into(), "--global".into(), "dotnet-ef".into()],
                cwd: Some(IpcPath::from(dir.to_path_buf())),
            }),
        };
    };
    let startup = startup_project(project, candidates);
    let mut args = prefix;
    args.extend(ef_args(op, project, &startup));
    EfCommand::Ready { command, args, cwd: IpcPath::from(dir.to_path_buf()), startup_project: IpcPath::from(startup) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_names_from_files() {
        assert_eq!(migration_name("20240101120000_InitialCreate.cs"), Some("InitialCreate"));
        assert_eq!(migration_name("20240101120000_InitialCreate.Designer.cs"), None);
        assert_eq!(migration_name("AppDbContextModelSnapshot.cs"), None);
        assert_eq!(migration_name("2024_Short.cs"), None);
        let dir = tempfile::tempdir().unwrap();
        let m = dir.path().join("Data/Migrations");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::create_dir_all(dir.path().join("obj")).unwrap();
        for f in [
            "20240202000000_AddOrders.cs",
            "20240101000000_InitialCreate.cs",
            "20240101000000_InitialCreate.Designer.cs",
            "AppDbContextModelSnapshot.cs",
        ] {
            std::fs::write(m.join(f), "").unwrap();
        }
        std::fs::write(dir.path().join("obj/20250101000000_Generated.cs"), "").unwrap();
        assert_eq!(migrations(dir.path()), ["InitialCreate", "AddOrders"]);
    }

    #[test]
    fn command_lines() {
        let (p, s) = (Path::new("/s/Data/Data.csproj"), Path::new("/s/Web/Web.csproj"));
        let tail = ["--project", "/s/Data/Data.csproj", "--startup-project", "/s/Web/Web.csproj"];
        let add = ef_args(
            &EfOperation::AddMigration { name: "Init".into(), output_dir: Some("Data/Migrations".into()) },
            p,
            s,
        );
        assert_eq!(add[..5], ["migrations", "add", "Init", "--output-dir", "Data/Migrations"]);
        assert_eq!(add[5..], tail);
        assert_eq!(
            ef_args(&EfOperation::UpdateDatabase { migration: Some("0".into()) }, p, s)[..3],
            ["database", "update", "0"]
        );
        assert_eq!(ef_args(&EfOperation::UpdateDatabase { migration: None }, p, s).len(), 6);
        let scaffold = ef_args(
            &EfOperation::ScaffoldDbContext {
                connection: "Data Source=app.db".into(),
                provider: "Microsoft.EntityFrameworkCore.Sqlite".into(),
                output_dir: Some("Models".into()),
                context: None,
            },
            p,
            s,
        );
        assert_eq!(
            scaffold[..6],
            [
                "dbcontext",
                "scaffold",
                "Data Source=app.db",
                "Microsoft.EntityFrameworkCore.Sqlite",
                "--output-dir",
                "Models"
            ]
        );
    }

    #[test]
    fn startup_project_is_an_executable_that_references_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        };
        let data = write("Data/Data.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"></Project>"#);
        let other = write("Other/Other.csproj", r#"<Project Sdk="Microsoft.NET.Sdk.Web"></Project>"#);
        let web = write(
            "Web/Web.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><ProjectReference Include="..\Data\Data.csproj" /></ItemGroup></Project>"#,
        );
        assert_eq!(startup_project(&data, &[other.clone(), web.clone()]), web);
        assert_eq!(startup_project(&web, std::slice::from_ref(&other)), web, "an executable starts itself");
        assert_eq!(startup_project(&data, &[other]), data, "nothing references it");
    }
}
