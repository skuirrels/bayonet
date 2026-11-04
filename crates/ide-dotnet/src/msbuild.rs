//! MSBuild navigation into the .NET SDK (CI-18): where the SDK in use defines a
//! target (`Build`, `Restore`, `CoreCompile`, …), for Go to Declaration on target
//! references in project files.

use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// A position in a file (0-based line and UTF-16 column, as the editor wants).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileLocation {
    pub path: IpcPath,
    pub line: u32,
    pub col: u32,
}

/// The SDK folder (`<dotnet home>/sdk/<version>`) that `dotnet` uses in `root` (honours `global.json`).
pub fn sdk_dir(dotnet: &Path, root: &Path) -> Option<PathBuf> {
    let out = ide_core::process::prepare(&mut std::process::Command::new(dotnet))
        .arg("--version")
        .current_dir(root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    // `dotnet` on PATH is usually a symlink into the install.
    let host = which(dotnet).and_then(|p| std::fs::canonicalize(p).ok())?;
    let dir = host.parent()?.join("sdk").join(version);
    dir.is_dir().then_some(dir)
}

fn which(cmd: &Path) -> Option<PathBuf> {
    if cmd.components().count() > 1 {
        return Some(cmd.to_path_buf());
    }
    let exe = if cfg!(windows) { format!("{}.exe", cmd.display()) } else { cmd.display().to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// The MSBuild files of an SDK folder: its own `.targets`/`.props` and those of every SDK under `Sdks/`.
pub fn sdk_files(sdk: &Path) -> Vec<PathBuf> {
    fn msbuild_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("targets") || e.eq_ignore_ascii_case("props"))
            })
            .collect();
        // `Microsoft.Common.CurrentVersion.targets` defines the build of one target framework;
        // the cross-targeting files (outer builds of multi-targeting projects) come last.
        files.sort_by_key(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (!name.contains("CurrentVersion"), name.contains("CrossTargeting"), name)
        });
        out.extend(files);
    }
    let mut out = Vec::new();
    // Microsoft.Common.CurrentVersion.targets and friends come first: they define the build.
    msbuild_files(sdk, &mut out);
    if let Ok(sdks) = std::fs::read_dir(sdk.join("Sdks")) {
        let mut sdks: Vec<PathBuf> = sdks.flatten().map(|e| e.path()).collect();
        sdks.sort();
        for s in sdks {
            msbuild_files(&s.join("Sdk"), &mut out);
            msbuild_files(&s.join("targets"), &mut out);
        }
    }
    out
}

/// Where `<Target Name="name" …>` is in `text` (names ignore case, as MSBuild's do).
pub fn find_target_in(text: &str, name: &str) -> Option<(u32, u32)> {
    let mut from = 0;
    while let Some(i) = text[from..].find("<Target") {
        let start = from + i;
        from = start + "<Target".len();
        // `<Target ` or `<Target\n`, not `<TargetFramework>`.
        if !text[from..].starts_with(char::is_whitespace) {
            continue;
        }
        let end = text[from..].find('>').map_or(text.len(), |e| from + e);
        let tag = &text[from..end];
        let value = tag.find("Name").and_then(|n| {
            let rest = tag[n + 4..].trim_start().strip_prefix('=')?.trim_start();
            let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'')?;
            rest[1..].split(quote).next()
        });
        if value.is_some_and(|v| v.trim().eq_ignore_ascii_case(name)) {
            let before = &text[..start];
            let line = before.matches('\n').count() as u32;
            let col = before.rsplit('\n').next().map_or(0, |l| l.encode_utf16().count()) as u32;
            return Some((line, col));
        }
    }
    None
}

/// The first of `files` that defines target `name`.
pub fn find_target(files: &[PathBuf], name: &str) -> Option<FileLocation> {
    files.iter().find_map(|f| {
        let text = std::fs::read_to_string(f).ok()?;
        let (line, col) = find_target_in(&text, name)?;
        Some(FileLocation { path: IpcPath::from(f.clone()), line, col })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_target_definitions() {
        let text = "<Project>\n  <PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup>\n  <Target\n      Name=\"Build\" DependsOnTargets=\"$(BuildDependsOn)\" />\n  <Target Name='CoreCompile'>\n</Project>";
        assert_eq!(find_target_in(text, "build"), Some((2, 2)));
        assert_eq!(find_target_in(text, "CoreCompile"), Some((4, 2)));
        assert_eq!(find_target_in(text, "TargetFramework"), None);
        assert_eq!(find_target_in(text, "Missing"), None);
    }

    /// Needs a .NET SDK on PATH.
    #[test]
    #[ignore = "uses the installed .NET SDK"]
    fn finds_build_in_the_installed_sdk() {
        let root = std::env::temp_dir();
        let sdk = sdk_dir(Path::new("dotnet"), &root).expect("an SDK");
        let loc = find_target(&sdk_files(&sdk), "Build").expect("Build is defined");
        assert!(loc.path.to_string_lossy().ends_with("Microsoft.Common.CurrentVersion.targets"), "{loc:?}");
    }
}
