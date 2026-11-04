//! What a project's analysers and source generators are, and the files the
//! generators write (CI-17), for the Solution Explorer's Dependencies › Analyzers.
//!
//! Roslyn keeps generated sources in memory, so the IDE compiles the project a
//! second time with `EmitCompilerGeneratedFiles` into its own cache: an import
//! scoped to the project (`CustomAfterMicrosoftCommonTargets`, conditioned on
//! the project path so referenced projects keep their outputs) moves the
//! intermediate folder there, which leaves the project's `obj` and `bin` alone.
//! The compiler writes `<out>/<generator assembly>/<generator type>/<hint name>`.
use ide_core::IpcPath;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// A project's analysers and what its source generators wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedSources {
    /// `<generator assembly>/<generator type>/<hint name>` below it; one folder per analyser.
    pub dir: IpcPath,
    /// Analyser assemblies, by name.
    pub analyzers: Vec<String>,
    pub files: Vec<IpcPath>,
    /// Compiler errors of the side compile (generated files may be missing then).
    pub errors: Vec<String>,
}

/// The import that redirects the compile of `project` (and only it) into the cache.
pub fn targets_file(project: &Path, obj: &Path, out: &Path) -> String {
    let esc = |p: &Path| {
        p.to_string_lossy()
            // MSBuild unescapes `%xx`, and expands `$(…)`, `@(…)` and `%(…)`; escape their markers.
            .replace('%', "%25")
            .replace('$', "%24")
            .replace('@', "%40")
            .replace(';', "%3B")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "&apos;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<Project>
  <PropertyGroup Condition="'$(MSBuildProjectFullPath)' == '{project}'">
    <IntermediateOutputPath>{obj}/</IntermediateOutputPath>
    <EmitCompilerGeneratedFiles>true</EmitCompilerGeneratedFiles>
    <CompilerGeneratedFilesOutputPath>{out}</CompilerGeneratedFilesOutputPath>
  </PropertyGroup>
</Project>
"#,
        project = esc(project),
        obj = esc(obj),
        out = esc(out),
    )
}

/// The `Analyzer` items of `dotnet msbuild -getItem:Analyzer` (after any errors the build printed).
pub fn parse_analyzers(stdout: &str) -> Vec<PathBuf> {
    let Some(start) = stdout.find("{\n").or_else(|| stdout.find('{')) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout[start..]) else { return Vec::new() };
    let mut out: Vec<PathBuf> = json["Items"]["Analyzer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["FullPath"].as_str().or_else(|| i["Identity"].as_str()))
        .map(PathBuf::from)
        .collect();
    out.dedup();
    out
}

/// The compiler's error lines (`… error CS1234: …`), without the trailing project.
pub fn errors(output: &str) -> Vec<String> {
    let mut out: Vec<String> = output
        .lines()
        .filter(|l| l.contains(": error "))
        .map(|l| match l.rfind(" [") {
            Some(i) if l.ends_with(']') => l[..i].trim().to_owned(),
            _ => l.trim().to_owned(),
        })
        .collect();
    out.dedup();
    out
}

/// One folder per analyser assembly under `out`, so analysers that generate
/// nothing (or only analyse) are listed too; returns their names, sorted.
pub fn add_analyzer_folders(out: &Path, analyzers: &[PathBuf]) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = analyzers
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    for n in &names {
        std::fs::create_dir_all(out.join(n))?;
    }
    Ok(names)
}

/// Generated files under `out`, sorted.
pub fn generated_files(out: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, into);
            } else {
                into.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(out, &mut files);
    files.sort();
    files
}

/// Marks the generated files read-only, so the editor opens them that way.
pub fn make_read_only(files: &[PathBuf]) {
    for f in files {
        if let Ok(meta) = std::fs::metadata(f) {
            let mut perms = meta.permissions();
            perms.set_readonly(true);
            let _ = std::fs::set_permissions(f, perms);
        }
    }
}

/// Removes an earlier listing (its files are read-only, which Windows refuses to delete).
pub fn remove_listing(dir: &Path) {
    #[allow(clippy::permissions_set_readonly_false)]
    for f in generated_files(dir) {
        if let Ok(meta) = std::fs::metadata(&f) {
            let mut perms = meta.permissions();
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(&f, perms);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the generated files of `project` go under `base`: the project file's
/// absolute path below `base/out`, so the client can work it out too.
pub fn output_dir(base: &Path, project: &Path) -> PathBuf {
    let rel: PathBuf = project
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(PathBuf::from(s)),
            std::path::Component::Prefix(p) => {
                Some(PathBuf::from(p.as_os_str().to_string_lossy().replace([':', '\\', '/', '?'], "")))
            }
            _ => None,
        })
        .collect();
    base.join("out").join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_the_import_to_the_project() {
        let t = targets_file(Path::new("/s/A&B/App.csproj"), Path::new("/c/obj"), Path::new("/c/$out"));
        assert!(t.contains("== '/s/A&amp;B/App.csproj'"));
        assert!(t.contains("<IntermediateOutputPath>/c/obj/</IntermediateOutputPath>"));
        assert!(t.contains("<CompilerGeneratedFilesOutputPath>/c/%24out</CompilerGeneratedFilesOutputPath>"));
    }

    #[test]
    fn reads_analyzer_items_after_errors() {
        let out = "CSC : error CS0006: Metadata file 'x' could not be found [/s/App.csproj]\n{\n  \"Items\": {\n    \"Analyzer\": [\n      { \"Identity\": \"/sdk/a/Microsoft.AspNetCore.Analyzers.dll\", \"FullPath\": \"/sdk/a/Microsoft.AspNetCore.Analyzers.dll\" },\n      { \"Identity\": \"../Gen/bin/Debug/netstandard2.0/Fixture.Generators.dll\", \"FullPath\": \"/s/Gen/bin/Debug/netstandard2.0/Fixture.Generators.dll\" }\n    ]\n  }\n}\n";
        assert_eq!(
            parse_analyzers(out),
            vec![
                PathBuf::from("/sdk/a/Microsoft.AspNetCore.Analyzers.dll"),
                PathBuf::from("/s/Gen/bin/Debug/netstandard2.0/Fixture.Generators.dll")
            ]
        );
        assert_eq!(errors(out), vec!["CSC : error CS0006: Metadata file 'x' could not be found"]);
        assert!(parse_analyzers("Build FAILED.").is_empty());
    }

    #[test]
    fn mirrors_the_project_path() {
        assert_eq!(
            output_dir(Path::new("/c/generated"), Path::new("/s/src/App/App.csproj")),
            PathBuf::from("/c/generated/out/s/src/App/App.csproj")
        );
    }

    #[test]
    fn lists_every_analyzer_and_the_generated_files() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        std::fs::create_dir_all(out.join("Gen/Gen.Names")).unwrap();
        std::fs::write(out.join("Gen/Gen.Names/Names.g.cs"), "class Names {}").unwrap();
        let names = add_analyzer_folders(
            out,
            &[PathBuf::from("/p/Gen.dll"), PathBuf::from("/sdk/Analyzers.dll"), PathBuf::from("/sdk/Analyzers.dll")],
        )
        .unwrap();
        assert_eq!(names, vec!["Analyzers", "Gen"]);
        assert!(out.join("Analyzers").is_dir());
        let files = generated_files(out);
        assert_eq!(files, vec![out.join("Gen/Gen.Names/Names.g.cs")]);
        make_read_only(&files);
        assert!(std::fs::metadata(&files[0]).unwrap().permissions().readonly());
        remove_listing(out);
        assert!(!out.exists());
    }
}
