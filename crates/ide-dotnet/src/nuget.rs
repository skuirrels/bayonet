//! NuGet through the SDK's own commands: `dotnet package search / list / add / remove`.
//!
//! Going through the SDK rather than a NuGet client of our own means `nuget.config`,
//! central package management (`Directory.Packages.props`), package source mapping
//! and the user's credential providers all behave exactly as on the command line:
//! installing into a CPM project writes the version to `Directory.Packages.props`.

use ide_core::error::ErrorKind;
use ide_core::{IdeError, IdeResult, IpcPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One page of search results, per package source.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackageSearch {
    pub sources: Vec<SourceResults>,
    /// Errors that are not one source's: a broken `nuget.config`, say.
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceResults {
    /// The source's name in `nuget.config` (`nuget.org`), else its URL.
    pub source: String,
    pub packages: Vec<PackageSummary>,
    /// Why this source returned nothing: unreachable, unauthorised.
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackageSummary {
    pub id: String,
    /// Latest version, or for an exact-match search the version of this entry.
    pub version: String,
    pub description: Option<String>,
    pub owners: Option<String>,
    pub total_downloads: Option<f64>,
    pub project_url: Option<String>,
}

/// The packages each project references, from `dotnet package list`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackages {
    pub projects: Vec<ProjectPackages>,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPackages {
    pub path: IpcPath,
    pub frameworks: Vec<FrameworkPackages>,
    /// The `Directory.Packages.props` that holds this project's versions (central package management).
    pub central_packages: Option<IpcPath>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkPackages {
    pub framework: String,
    pub packages: Vec<InstalledPackage>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    pub id: String,
    /// The version the project (or `Directory.Packages.props`) asks for: `13.0.3`, `[1.0,2.0)`.
    pub requested: Option<String>,
    /// The version restore picked.
    pub resolved: Option<String>,
    /// Only from an `outdated` listing: the newest version on the sources.
    pub latest: Option<String>,
    /// Implicit references the SDK adds (`NETStandard.Library`); they cannot be removed.
    pub auto_referenced: bool,
}

/// `dotnet package search <query>` with descriptions; `skip` / `take` page through results.
pub fn search_args(query: &str, prerelease: bool, skip: u32, take: u32) -> Vec<String> {
    let mut args = vec!["package".into(), "search".into()];
    if !query.trim().is_empty() {
        args.push(query.trim().into());
    }
    args.extend(["--skip".into(), skip.to_string(), "--take".into(), take.to_string()]);
    args.extend(["--format".into(), "json".into(), "--verbosity".into(), "detailed".into()]);
    if prerelease {
        args.push("--prerelease".into());
    }
    args
}

/// Every version of package `id` on the sources.
pub fn versions_args(id: &str, prerelease: bool) -> Vec<String> {
    let mut args: Vec<String> = ["package", "search", id, "--exact-match", "--format", "json"].map(String::from).into();
    if prerelease {
        args.push("--prerelease".into());
    }
    args
}

/// `dotnet package list` of a project or a whole solution; `outdated` adds the latest versions (and needs the network).
pub fn list_args(target: &Path, outdated: bool, prerelease: bool) -> Vec<String> {
    let mut args = vec!["package".into(), "list".into(), "--project".into(), path_arg(target)];
    args.extend(["--format".into(), "json".into()]);
    if outdated {
        args.push("--outdated".into());
        if prerelease {
            args.push("--include-prerelease".into());
        }
    }
    args
}

/// Installs package `id` (at `version`, else the latest stable) into `project`, or
/// moves it to `version` when it is there already.
pub fn add_args(project: &Path, id: &str, version: Option<&str>) -> Vec<String> {
    let package = match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("{id}@{v}"),
        None => id.to_owned(),
    };
    vec!["package".into(), "add".into(), package, "--project".into(), path_arg(project)]
}

pub fn remove_args(project: &Path, id: &str) -> Vec<String> {
    vec!["package".into(), "remove".into(), id.into(), "--project".into(), path_arg(project)]
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Runs `dotnet <args>` in `cwd` with telemetry, banners and localisation off; stdout on success.
pub fn run(dotnet: Option<&Path>, cwd: &Path, args: &[String]) -> IdeResult<String> {
    let program = dotnet.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("dotnet"));
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .current_dir(cwd)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1");
    if let Some(root) = dotnet.and_then(Path::parent) {
        cmd.env("DOTNET_ROOT", root);
    }
    let out = cmd.output().map_err(|e| IdeError::unavailable(format!("cannot run {}: {e}", program.display())))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    // JSON output still describes what went wrong; let the parser report it.
    if stdout.trim_start().starts_with('{') {
        return Ok(stdout);
    }
    Err(IdeError::new(ErrorKind::Io, failure(&stdout, &String::from_utf8_lossy(&out.stderr))))
}

/// The lines of a failed `dotnet` run that say why: errors first, else the last output.
pub fn failure(stdout: &str, stderr: &str) -> String {
    let lines: Vec<&str> = stdout.lines().chain(stderr.lines()).map(str::trim).filter(|l| !l.is_empty()).collect();
    let errors: Vec<&str> =
        lines.iter().copied().filter(|l| l.starts_with("error") || l.contains(": error ")).collect();
    let pick = if errors.is_empty() { lines.iter().rev().take(3).rev().copied().collect() } else { errors };
    if pick.is_empty() { "dotnet failed".to_owned() } else { pick.join("\n") }
}

fn json(text: &str) -> IdeResult<Value> {
    let start = text.find('{').ok_or_else(|| IdeError::new(ErrorKind::Io, failure(text, "")))?;
    serde_json::from_str(&text[start..])
        .map_err(|e| IdeError::new(ErrorKind::Protocol, format!("unexpected dotnet output: {e}")))
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned).filter(|s| !s.is_empty())
}

/// `problems`: strings, or `{ text, problemType }` objects.
fn problems(v: &Value) -> Vec<String> {
    v.get("problems")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned).or_else(|| str_of(p, "text"))).collect())
        .unwrap_or_default()
}

fn array<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

/// The JSON of `dotnet package search --format json`.
pub fn parse_search(text: &str) -> IdeResult<PackageSearch> {
    let v = json(text)?;
    let sources = array(&v, "searchResult")
        .map(|s| SourceResults {
            source: str_of(s, "sourceName").unwrap_or_default(),
            packages: array(s, "packages")
                .filter_map(|p| {
                    Some(PackageSummary {
                        id: str_of(p, "id")?,
                        version: str_of(p, "latestVersion").or_else(|| str_of(p, "version")).unwrap_or_default(),
                        description: str_of(p, "description"),
                        owners: str_of(p, "owners"),
                        total_downloads: p.get("totalDownloads").and_then(Value::as_f64),
                        project_url: str_of(p, "projectUrl"),
                    })
                })
                .collect(),
            problems: problems(s),
        })
        .collect();
    Ok(PackageSearch { sources, problems: problems(&v) })
}

/// The versions in an exact-match search, newest first, without duplicates across sources.
pub fn parse_versions(text: &str) -> IdeResult<Vec<String>> {
    let search = parse_search(text)?;
    let mut versions: Vec<String> = Vec::new();
    for p in search.sources.iter().flat_map(|s| &s.packages) {
        if !p.version.is_empty() && !versions.iter().any(|v| v.eq_ignore_ascii_case(&p.version)) {
            versions.push(p.version.clone());
        }
    }
    versions.sort_by(|a, b| compare_versions(b, a));
    Ok(versions)
}

/// The JSON of `dotnet package list --format json`.
pub fn parse_list(text: &str) -> IdeResult<InstalledPackages> {
    let v = json(text)?;
    let mut problems = problems(&v);
    let projects = array(&v, "projects")
        .map(|p| {
            problems.extend(self::problems(p));
            ProjectPackages {
                path: IpcPath::from(str_of(p, "path").unwrap_or_default()),
                frameworks: array(p, "frameworks")
                    .map(|f| FrameworkPackages {
                        framework: str_of(f, "framework").unwrap_or_default(),
                        packages: array(f, "topLevelPackages")
                            .filter_map(|x| {
                                Some(InstalledPackage {
                                    id: str_of(x, "id")?,
                                    requested: str_of(x, "requestedVersion"),
                                    resolved: str_of(x, "resolvedVersion"),
                                    latest: str_of(x, "latestVersion"),
                                    auto_referenced: matches!(x.get("autoReferenced"), Some(Value::Bool(true)))
                                        || str_of(x, "autoReferenced").is_some_and(|s| s.eq_ignore_ascii_case("true")),
                                })
                            })
                            .collect(),
                    })
                    .collect(),
                central_packages: None,
            }
        })
        .collect();
    Ok(InstalledPackages { projects, problems })
}

/// NuGet version order: numeric parts, then a release sorts after its prereleases,
/// prerelease labels compared part by part (numbers numerically). Build metadata is ignored.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let mut nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        while nums.len() < 4 {
            nums.push(0);
        }
        (nums, pre)
    }
    let ((an, ap), (bn, bp)) = (split(a), split(b));
    an.cmp(&bn).then_with(|| match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (mut xs, mut ys) = (x.split('.'), y.split('.'));
            loop {
                match (xs.next(), ys.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(p), Some(q)) => {
                        let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                            (Ok(m), Ok(n)) => m.cmp(&n),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            _ => p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase()),
                        };
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                }
            }
        }
    })
}

/// `Directory.Packages.props` governing `project`: the nearest one in its folder or
/// above, when it turns central package management on.
pub fn central_packages_file(project: &Path) -> Option<PathBuf> {
    let file = project.parent()?.ancestors().map(|d| d.join("Directory.Packages.props")).find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&file).ok()?;
    let on = text.split("<ManagePackageVersionsCentrally>").nth(1).and_then(|t| t.split('<').next());
    on.is_some_and(|v| v.trim().eq_ignore_ascii_case("true")).then_some(file)
}

/// Fills in each project's central package file.
pub fn with_central_packages(mut list: InstalledPackages) -> InstalledPackages {
    for p in &mut list.projects {
        p.central_packages = central_packages_file(&p.path).map(IpcPath::from);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_per_source_with_problems() {
        let text = r#"{"version":2,"problems":[],"searchResult":[
            {"sourceName":"nuget.org","packages":[{"id":"Humanizer","latestVersion":"3.0.10","totalDownloads":173894242,
              "owners":"dotnetfoundation, Humanizer","description":"Strings","projectUrl":"https://github.com/Humanizr/Humanizer"}]},
            {"sourceName":"https://example.invalid/v3/index.json","problems":[{"text":"Unable to load the service index","problemType":"Error"}],"packages":[]}]}"#;
        let s = parse_search(text).unwrap();
        assert_eq!(s.sources.len(), 2);
        let h = &s.sources[0].packages[0];
        assert_eq!((h.id.as_str(), h.version.as_str()), ("Humanizer", "3.0.10"));
        assert_eq!(h.total_downloads, Some(173894242.0));
        assert_eq!(h.owners.as_deref(), Some("dotnetfoundation, Humanizer"));
        assert_eq!(s.sources[1].problems, ["Unable to load the service index"]);
    }

    #[test]
    fn exact_match_versions_newest_first_once() {
        let text = r#"{"version":2,"problems":[],"searchResult":[
            {"sourceName":"a","packages":[{"id":"X","version":"2.0.0"},{"id":"X","version":"10.0.0-preview.1"},{"id":"X","version":"10.0.0"}]},
            {"sourceName":"b","packages":[{"id":"X","version":"2.0.0"},{"id":"X","version":"9.1.0"}]}]}"#;
        assert_eq!(parse_versions(text).unwrap(), ["10.0.0", "10.0.0-preview.1", "9.1.0", "2.0.0"]);
    }

    #[test]
    fn list_of_a_solution() {
        let text = r#"{"version":1,"parameters":"--outdated","projects":[
            {"path":"/s/src/Api/Api.csproj","frameworks":[{"framework":"net10.0","topLevelPackages":[
               {"id":"Microsoft.AspNetCore.OpenApi","requestedVersion":"10.0.14","resolvedVersion":"10.0.14","latestVersion":"10.0.15"}]}]},
            {"path":"/s/src/Core/Core.csproj","frameworks":[{"framework":"netstandard2.0","topLevelPackages":[
               {"id":"NETStandard.Library","requestedVersion":"[2.0.3, )","resolvedVersion":"2.0.3","autoReferenced":"true"}]}]},
            {"path":"/s/src/Cli/Cli.csproj","problems":[{"text":"not restored","level":"error"}]}]}"#;
        let l = parse_list(text).unwrap();
        assert_eq!(l.projects.len(), 3);
        let api = &l.projects[0].frameworks[0].packages[0];
        assert_eq!((api.resolved.as_deref(), api.latest.as_deref()), (Some("10.0.14"), Some("10.0.15")));
        assert!(!api.auto_referenced);
        assert!(l.projects[1].frameworks[0].packages[0].auto_referenced);
        assert!(l.projects[2].frameworks.is_empty());
        assert_eq!(l.problems, ["not restored"]);
    }

    #[test]
    fn non_json_output_is_an_error_with_its_reason() {
        let e = parse_list("error: Could not find project or directory `x`.\n").unwrap_err();
        assert!(e.message.contains("Could not find project"), "{}", e.message);
    }

    #[test]
    fn command_lines() {
        let p = Path::new("/s/src/Api/Api.csproj");
        assert_eq!(
            add_args(p, "Humanizer", Some("3.0.10")),
            ["package", "add", "Humanizer@3.0.10", "--project", "/s/src/Api/Api.csproj"]
        );
        assert_eq!(
            add_args(p, "Humanizer", Some(" ")),
            ["package", "add", "Humanizer", "--project", "/s/src/Api/Api.csproj"]
        );
        assert_eq!(
            remove_args(p, "Humanizer"),
            ["package", "remove", "Humanizer", "--project", "/s/src/Api/Api.csproj"]
        );
        assert_eq!(
            list_args(Path::new("/s/S.slnx"), true, true),
            ["package", "list", "--project", "/s/S.slnx", "--format", "json", "--outdated", "--include-prerelease"]
        );
        let s = search_args(" json ", true, 20, 20);
        assert_eq!(&s[..3], ["package", "search", "json"]);
        assert!(s.contains(&"--prerelease".to_owned()) && s.contains(&"detailed".to_owned()));
        assert!(!search_args("", false, 0, 20).contains(&"".to_owned()));
    }

    #[test]
    fn version_order() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Greater);
        assert_eq!(compare_versions("1.0", "1.0.0.0"), Equal);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Less);
        assert_eq!(compare_versions("1.0.0-beta.10", "1.0.0-beta.9"), Greater);
        assert_eq!(compare_versions("1.0.0-alpha", "1.0.0-alpha.1"), Less);
        assert_eq!(compare_versions("1.0.0+abc", "1.0.0"), Equal);
    }

    #[test]
    fn central_package_file_is_the_nearest_above() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/A")).unwrap();
        let props = dir.path().join("Directory.Packages.props");
        std::fs::write(&props, "<Project/>").unwrap();
        assert_eq!(central_packages_file(&dir.path().join("src/A/A.csproj")), None, "CPM is off");
        std::fs::write(&props, "<Project><PropertyGroup><ManagePackageVersionsCentrally> true </ManagePackageVersionsCentrally></PropertyGroup></Project>").unwrap();
        let found = central_packages_file(&dir.path().join("src/A/A.csproj")).unwrap();
        assert_eq!(found, dir.path().join("Directory.Packages.props"));
    }

    /// Real SDK: lists the fixture solution. Needs `dotnet` on PATH.
    #[test]
    #[ignore]
    fn lists_the_fixture_solution() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet").canonicalize().unwrap();
        let out = run(None, &root, &list_args(&root.join("DotnetFixture.slnx"), false, false)).unwrap();
        let l = parse_list(&out).unwrap();
        let ids: Vec<&str> =
            l.projects.iter().flat_map(|p| &p.frameworks).flat_map(|f| &f.packages).map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"xunit.v3") && ids.contains(&"Microsoft.AspNetCore.OpenApi"), "{ids:?}");
    }
}
