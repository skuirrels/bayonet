//! What a project's analysers and source generators are, and the files the
//! generators write (CI-17), for the Solution Explorer's Dependencies › Analyzers.
//!
//! Roslyn keeps generated sources in memory, so the IDE compiles the project a
//! second time with `EmitCompilerGeneratedFiles` into its own cache: an import
//! scoped to the project (`CustomAfterMicrosoftCommonTargets`, conditioned on
//! the project path so referenced projects keep their outputs) moves the
//! intermediate folder there, which leaves the project's `obj` and `bin` alone.
//! The project's own `CustomAfterMicrosoftCommonTargets` is evaluated first and
//! imported from that file, since the global property replaces it.
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
    /// The compile failed and produced nothing, so `files` are from an earlier compile.
    pub stale: bool,
    /// Referenced projects have not been built yet, so the compile cannot run.
    pub needs_build: bool,
}

/// Properties the side compile reads first: the user's own import (which the
/// global property would hide) and the target frameworks.
pub const EVAL_PROPERTIES: [&str; 3] = ["CustomAfterMicrosoftCommonTargets", "TargetFramework", "TargetFrameworks"];

/// Escapes `s` for a value or quoted condition literal in an MSBuild XML file.
fn msbuild_escape(s: &str) -> String {
    // MSBuild unescapes `%xx` and expands `$(…)`, `@(…)` and `%(…)`; a `'`
    // would end a condition literal (XML entities are undone before MSBuild sees it).
    s.replace('%', "%25")
        .replace('$', "%24")
        .replace('@', "%40")
        .replace(';', "%3B")
        .replace('\'', "%27")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The import that redirects the compile of `project` (and only it) into the
/// cache, after importing the project's own `CustomAfterMicrosoftCommonTargets`
/// (`user_import`), which the global property naming this file replaces.
pub fn targets_file(project: &Path, obj: &Path, out: &Path, user_import: Option<&str>) -> String {
    let esc = |p: &Path| msbuild_escape(&p.to_string_lossy());
    let project = esc(project);
    let import = user_import.map(str::trim).filter(|u| !u.is_empty()).map(msbuild_escape).map(|u| {
        format!(
            "  <Import Project=\"{u}\" Condition=\"'$(MSBuildProjectFullPath)' == '{project}' and Exists('{u}')\" />\n"
        )
    });
    format!(
        r#"<Project>
{import}  <PropertyGroup Condition="'$(MSBuildProjectFullPath)' == '{project}'">
    <IntermediateOutputPath>{obj}/</IntermediateOutputPath>
    <EmitCompilerGeneratedFiles>true</EmitCompilerGeneratedFiles>
    <CompilerGeneratedFilesOutputPath>{out}</CompilerGeneratedFilesOutputPath>
  </PropertyGroup>
</Project>
"#,
        import = import.unwrap_or_default(),
        obj = esc(obj),
        out = esc(out),
    )
}

/// Where MSBuild's JSON output starts: the first line that is just `{`
/// (error lines before it may contain braces; Windows ends lines with CRLF).
fn json_start(stdout: &str) -> Option<usize> {
    let mut at = 0;
    for line in stdout.split_inclusive('\n') {
        if line.trim() == "{" {
            return Some(at);
        }
        at += line.len();
    }
    None
}

/// The `Analyzer` items of `dotnet msbuild -getItem:Analyzer` (after any errors the build printed).
pub fn parse_analyzers(stdout: &str) -> Vec<PathBuf> {
    let Some(start) = json_start(stdout) else { return Vec::new() };
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

/// The properties of `dotnet msbuild -getProperty:…` (several names: a JSON object).
pub fn parse_properties(stdout: &str) -> Vec<(String, String)> {
    let Some(start) = json_start(stdout) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout[start..]) else { return Vec::new() };
    json["Properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
        .collect()
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

/// A referenced project's assembly is missing (`CS0006`): it has not been built.
pub fn needs_build(errors: &[String]) -> bool {
    errors.iter().any(|e| e.contains("CS0006"))
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

/// The analyser folders of a listing, sorted.
pub fn analyzer_names(out: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(out)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
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

/// Where the generated files of `project` go under `base`: `<name>-<hash of
/// its path>`, short enough for Windows paths once the compiler adds
/// `<generator assembly>/<generator type>/<hint name>`.
pub fn output_dir(base: &Path, project: &Path) -> PathBuf {
    // FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`.
    let hash = project
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let name: String = project.file_stem().map(|s| s.to_string_lossy().chars().take(40).collect()).unwrap_or_default();
    base.join("out").join(format!("{name}-{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_the_import_to_the_project() {
        let t = targets_file(Path::new("/s/A&B/App.csproj"), Path::new("/c/obj"), Path::new("/c/$out"), None);
        assert!(t.contains("== '/s/A&amp;B/App.csproj'"));
        assert!(t.contains("<IntermediateOutputPath>/c/obj/</IntermediateOutputPath>"));
        assert!(t.contains("<CompilerGeneratedFilesOutputPath>/c/%24out</CompilerGeneratedFilesOutputPath>"));
        assert!(!t.contains("<Import"));
        // A quote would end the condition's literal once XML is parsed.
        let t = targets_file(Path::new("/Users/o'brien/App.csproj"), Path::new("/c/obj"), Path::new("/c/out"), None);
        assert!(t.contains("== '/Users/o%27brien/App.csproj'"), "{t}");
    }

    #[test]
    fn keeps_the_projects_own_import() {
        let t = targets_file(
            Path::new("/s/App.csproj"),
            Path::new("/c/obj"),
            Path::new("/c/out"),
            Some("/s/build/after.targets"),
        );
        assert!(t.contains(
            "<Import Project=\"/s/build/after.targets\" Condition=\"'$(MSBuildProjectFullPath)' == '/s/App.csproj' and Exists('/s/build/after.targets')\" />"
        ));
        assert!(
            !targets_file(Path::new("/s/App.csproj"), Path::new("/o"), Path::new("/c"), Some("  ")).contains("<Import")
        );
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
        let errs = errors(out);
        assert_eq!(errs, vec!["CSC : error CS0006: Metadata file 'x' could not be found"]);
        assert!(needs_build(&errs));
        assert!(parse_analyzers("Build FAILED.").is_empty());
    }

    #[test]
    fn finds_the_json_past_braces_and_crlf() {
        let out = "Program.cs(3,1): error CS1514: { expected [/s/App.csproj]\r\n{\r\n  \"Items\": { \"Analyzer\": [ { \"FullPath\": \"C:\\\\sdk\\\\A.dll\" } ] }\r\n}\r\n";
        assert_eq!(parse_analyzers(out), vec![PathBuf::from("C:\\sdk\\A.dll")]);
        let props = "{\r\n  \"Properties\": { \"TargetFrameworks\": \"net10.0;netstandard2.0\", \"CustomAfterMicrosoftCommonTargets\": \"\" }\r\n}\r\n";
        assert_eq!(
            parse_properties(props),
            vec![
                ("CustomAfterMicrosoftCommonTargets".to_owned(), String::new()),
                ("TargetFrameworks".to_owned(), "net10.0;netstandard2.0".to_owned())
            ]
        );
    }

    #[test]
    fn names_the_folder_after_the_project_with_a_stable_hash() {
        let a = output_dir(Path::new("/c/generated"), Path::new("/s/src/App/App.csproj"));
        assert_eq!(a, output_dir(Path::new("/c/generated"), Path::new("/s/src/App/App.csproj")));
        assert_ne!(a, output_dir(Path::new("/c/generated"), Path::new("/t/src/App/App.csproj")));
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(a.starts_with("/c/generated/out") && name.starts_with("App-") && name.len() == 4 + 16, "{name}");
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
        assert_eq!(analyzer_names(out), names);
        assert!(out.join("Analyzers").is_dir());
        let files = generated_files(out);
        assert_eq!(files, vec![out.join("Gen/Gen.Names/Names.g.cs")]);
        make_read_only(&files);
        assert!(std::fs::metadata(&files[0]).unwrap().permissions().readonly());
        remove_listing(out);
        assert!(!out.exists());
    }
}
