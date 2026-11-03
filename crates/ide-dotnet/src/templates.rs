//! `dotnet new` project templates for the New Project dialog (SLN-7), and the
//! solution and reference edits that go with creating a project (SLN-4).
//!
//! Templates come from the template engine's cache that `dotnet new` keeps per
//! SDK version (`~/.templateengine/dotnetcli/<sdk>/templatecache.json`): every
//! installed template, custom ones included, with its parameters, so the list
//! needs no `dotnet new list` table parsing (whose columns truncate names).
//! Creating, adding to the solution and editing references go through
//! `dotnet new`, `dotnet sln` and `dotnet reference`, which keep the files'
//! formatting (the SDK edits solutions with SolutionPersistence and projects
//! with MSBuild's construction model).

use ide_core::{IdeError, IdeResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTemplate {
    /// The group identity (one entry for all languages and versions of a template).
    pub id: String,
    pub name: String,
    /// What `dotnet new` takes: `console`, `webapi`.
    pub short_name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    /// Classifications: `Common`, `Web`, `Test`.
    pub tags: Vec<String>,
    /// `C#` first when offered.
    pub languages: Vec<String>,
    /// `project` or `solution`.
    pub kind: String,
    /// The options the dialog shows, for the default language.
    pub parameters: Vec<TemplateParameter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateParameter {
    pub name: String,
    /// The command-line option: `--framework`.
    pub option: String,
    pub label: String,
    pub description: Option<String>,
    /// `choice`, `bool`, `string` / `text`, `int`…
    pub data_type: String,
    pub default_value: Option<String>,
    pub choices: Vec<TemplateChoice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateChoice {
    pub value: String,
    pub label: String,
}

/// One template option the user set: `--framework` = `net10.0`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateValue {
    pub option: String,
    pub value: String,
}

/// Parameters the dialog sets itself or that make no sense there.
const OWN_PARAMETERS: &[&str] = &["name", "language", "type", "skipRestore", "TargetFrameworkOverride"];

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned).filter(|s| !s.is_empty())
}

/// The template cache of SDK `sdk_version` under `home`.
pub fn cache_file(home: &Path, sdk_version: &str) -> PathBuf {
    home.join(".templateengine").join("dotnetcli").join(sdk_version).join("templatecache.json")
}

fn language_rank(l: &str) -> u8 {
    match l {
        "C#" => 0,
        "F#" => 1,
        "VB" => 2,
        _ => 3,
    }
}

/// Project and solution templates of a `templatecache.json`, by name: for each
/// template group the highest-precedence variant of each language.
pub fn parse_cache(text: &str) -> IdeResult<Vec<ProjectTemplate>> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| IdeError::invalid(format!("unreadable template cache: {e}")))?;
    let infos = v.get("TemplateInfo").and_then(Value::as_array).ok_or_else(|| IdeError::invalid("no TemplateInfo"))?;
    // group → language → (precedence, entry)
    let mut groups: BTreeMap<String, BTreeMap<String, (i64, &Value)>> = BTreeMap::new();
    for t in infos {
        let tags = t.get("TagsCollection");
        let kind = tags.and_then(|g| str_of(g, "type")).unwrap_or_default();
        if kind != "project" && kind != "solution" {
            continue;
        }
        let Some(identity) = str_of(t, "Identity") else { continue };
        let group = str_of(t, "GroupIdentity").unwrap_or(identity);
        let language = tags.and_then(|g| str_of(g, "language")).unwrap_or_default();
        let precedence = t.get("Precedence").and_then(Value::as_i64).unwrap_or(0);
        let slot = groups.entry(group).or_default().entry(language).or_insert((i64::MIN, t));
        if precedence >= slot.0 {
            *slot = (precedence, t);
        }
    }
    let mut out: Vec<ProjectTemplate> = groups
        .into_iter()
        .filter_map(|(id, by_language)| {
            let mut languages: Vec<String> = by_language.keys().filter(|l| !l.is_empty()).cloned().collect();
            languages.sort_by_key(|l| (language_rank(l), l.clone()));
            let main = languages.first().and_then(|l| by_language.get(l)).or_else(|| by_language.values().next())?.1;
            Some(ProjectTemplate {
                id,
                name: str_of(main, "Name")?,
                short_name: main.get("ShortNameList").and_then(Value::as_array)?.first()?.as_str()?.to_owned(),
                description: str_of(main, "Description"),
                author: str_of(main, "Author"),
                tags: main
                    .get("Classifications")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default(),
                languages,
                kind: main.get("TagsCollection").and_then(|g| str_of(g, "type")).unwrap_or_default(),
                parameters: parameters(main),
            })
        })
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    Ok(out)
}

fn parameters(t: &Value) -> Vec<TemplateParameter> {
    let host: Value = str_of(t, "HostData").and_then(|h| serde_json::from_str(&h).ok()).unwrap_or(Value::Null);
    let symbols = host.get("symbolInfo");
    let Some(list) = t.get("Parameters").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|p| {
            let name = str_of(p, "Name")?;
            if OWN_PARAMETERS.contains(&name.as_str())
                || p.get("Type").and_then(Value::as_str).is_some_and(|k| k != "parameter")
            {
                return None;
            }
            let info = symbols.and_then(|s| s.get(&name));
            if info
                .and_then(|i| i.get("isHidden"))
                .is_some_and(|h| h.as_str() == Some("true") || h.as_bool() == Some(true))
            {
                return None;
            }
            let long = info.and_then(|i| str_of(i, "longName")).unwrap_or_else(|| name.clone());
            let choices = p
                .get("Choices")
                .and_then(Value::as_object)
                .map(|c| {
                    c.iter()
                        .map(|(value, d)| TemplateChoice {
                            value: value.clone(),
                            label: str_of(d, "DisplayName")
                                .or_else(|| str_of(d, "Description"))
                                .unwrap_or_else(|| value.clone()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(TemplateParameter {
                label: str_of(p, "DisplayName").unwrap_or_else(|| name.clone()),
                option: format!("--{long}"),
                description: str_of(p, "Description").or_else(|| str_of(p, "Documentation")),
                data_type: str_of(p, "DataType").unwrap_or_else(|| "string".into()),
                default_value: p.get("DefaultValue").and_then(Value::as_str).map(str::to_owned),
                choices,
                name,
            })
        })
        .collect()
}

/// `dotnet new <template> -n <name> -o <dir> [--language L] [--option value…]`.
/// Booleans are passed as `--option true|false`; empty values are left out.
pub fn new_args(
    template: &str,
    name: &str,
    output: &Path,
    language: Option<&str>,
    values: &[TemplateValue],
) -> Vec<String> {
    let mut args = vec!["new".into(), template.into(), "-n".into(), name.into(), "-o".into()];
    args.push(output.to_string_lossy().into_owned());
    if let Some(l) = language.filter(|l| !l.is_empty()) {
        args.extend(["--language".into(), l.into()]);
    }
    for TemplateValue { option, value } in values {
        if option.starts_with("--") && !value.is_empty() {
            args.extend([option.clone(), value.clone()]);
        }
    }
    args
}

/// `dotnet sln <solution> add <projects…> [--solution-folder F]`.
pub fn sln_add_args(solution: &Path, projects: &[PathBuf], folder: Option<&str>) -> Vec<String> {
    let mut args = vec!["sln".into(), solution.to_string_lossy().into_owned(), "add".into()];
    args.extend(projects.iter().map(|p| p.to_string_lossy().into_owned()));
    if let Some(f) = folder.map(str::trim).filter(|f| !f.is_empty()) {
        args.extend(["--solution-folder".into(), f.into()]);
    }
    args
}

pub fn sln_remove_args(solution: &Path, projects: &[PathBuf]) -> Vec<String> {
    let mut args = vec!["sln".into(), solution.to_string_lossy().into_owned(), "remove".into()];
    args.extend(projects.iter().map(|p| p.to_string_lossy().into_owned()));
    args
}

/// `dotnet reference add|remove <references…> --project <project>`.
pub fn reference_args(add: bool, project: &Path, references: &[PathBuf]) -> Vec<String> {
    let mut args = vec!["reference".into(), if add { "add" } else { "remove" }.into()];
    args.extend(references.iter().map(|p| p.to_string_lossy().into_owned()));
    args.extend(["--project".into(), project.to_string_lossy().into_owned()]);
    args
}

/// The project file `dotnet new` created in `dir` (named after `name` when there are several).
pub fn created_project(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> =
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| crate::is_project_file(p)).collect();
    found.sort();
    found.iter().find(|p| p.file_stem().is_some_and(|s| s == name)).cloned().or_else(|| found.into_iter().next())
}

/// Absolute paths of `project`'s project references.
pub fn project_references(project: &Path) -> IdeResult<Vec<PathBuf>> {
    let info = crate::read_project(project)?;
    let dir = project.parent().unwrap_or(project);
    Ok(info
        .project_references
        .iter()
        .map(|r| {
            let joined = dir.join(r.replace('\\', "/"));
            std::fs::canonicalize(&joined).unwrap_or(joined)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CACHE: &str = r#"{"Version":"1","TemplateInfo":[
      {"Identity":"Microsoft.Common.Console.CSharp.9.0","GroupIdentity":"Microsoft.Common.Console","Precedence":11000,
       "Name":"Console App","ShortNameList":["console"],"Description":"old","Author":"Microsoft","Classifications":["Common","Console"],
       "TagsCollection":{"language":"C#","type":"project"},"Parameters":[]},
      {"Identity":"Microsoft.Common.Console.CSharp.10.0","GroupIdentity":"Microsoft.Common.Console","Precedence":12000,
       "Name":"Console App","ShortNameList":["console"],"Description":"A project for a command-line application","Author":"Microsoft",
       "Classifications":["Common","Console"],"TagsCollection":{"language":"C#","type":"project"},
       "HostData":"{\"symbolInfo\":{\"Framework\":{\"longName\":\"framework\"},\"TargetFrameworkOverride\":{\"isHidden\":\"true\"},\"UseProgramMain\":{\"longName\":\"use-program-main\"}}}",
       "Parameters":[
         {"Name":"name","Type":"parameter","DataType":"string"},
         {"Name":"Framework","Type":"parameter","DataType":"choice","DefaultValue":"net10.0","DisplayName":"Framework",
          "Description":"The target framework for the project.","Choices":{"net10.0":{"DisplayName":".NET 10.0"},"net9.0":{"Description":"Target net9.0"}}},
         {"Name":"UseProgramMain","Type":"parameter","DataType":"bool","DefaultValue":"false"},
         {"Name":"skipRestore","Type":"parameter","DataType":"bool"},
         {"Name":"HostIdentifier","Type":"bind","DataType":"string"}]},
      {"Identity":"Microsoft.Common.Console.FSharp.10.0","GroupIdentity":"Microsoft.Common.Console","Precedence":12000,
       "Name":"Console App","ShortNameList":["console"],"TagsCollection":{"language":"F#","type":"project"},"Parameters":[]},
      {"Identity":"Microsoft.Standard.QuickStarts.DirectoryProps","Name":"MSBuild Directory.Build.props file","ShortNameList":["buildprops"],
       "TagsCollection":{"type":"item"},"Parameters":[]},
      {"Identity":"Microsoft.Standard.QuickStarts.Solution","Name":"Solution File","ShortNameList":["sln","solution"],
       "TagsCollection":{"type":"solution"},"Parameters":[]}]}"#;

    #[test]
    fn templates_of_the_cache() {
        let t = parse_cache(CACHE).unwrap();
        assert_eq!(t.iter().map(|t| t.short_name.as_str()).collect::<Vec<_>>(), ["console", "sln"]);
        let console = &t[0];
        assert_eq!(console.languages, ["C#", "F#"]);
        assert_eq!(console.description.as_deref(), Some("A project for a command-line application"));
        assert_eq!(console.tags, ["Common", "Console"]);
        let names: Vec<_> = console.parameters.iter().map(|p| (p.name.as_str(), p.option.as_str())).collect();
        assert_eq!(names, [("Framework", "--framework"), ("UseProgramMain", "--use-program-main")]);
        let fw = &console.parameters[0];
        assert_eq!(fw.default_value.as_deref(), Some("net10.0"));
        assert_eq!(
            fw.choices.iter().map(|c| (c.value.as_str(), c.label.as_str())).collect::<Vec<_>>(),
            [("net10.0", ".NET 10.0"), ("net9.0", "Target net9.0")]
        );
    }

    #[test]
    fn command_lines() {
        let v = |o: &str, x: &str| TemplateValue { option: o.into(), value: x.into() };
        let values = vec![v("--framework", "net10.0"), v("--aot", ""), v("bad", "x")];
        assert_eq!(
            new_args("console", "App", Path::new("/s/src/App"), Some("C#"), &values),
            ["new", "console", "-n", "App", "-o", "/s/src/App", "--language", "C#", "--framework", "net10.0"]
        );
        assert_eq!(
            sln_add_args(Path::new("/s/S.slnx"), &[PathBuf::from("/s/src/App/App.csproj")], Some("src")),
            ["sln", "/s/S.slnx", "add", "/s/src/App/App.csproj", "--solution-folder", "src"]
        );
        assert_eq!(
            reference_args(true, Path::new("/s/A.csproj"), &[PathBuf::from("/s/B.csproj")]),
            ["reference", "add", "/s/B.csproj", "--project", "/s/A.csproj"]
        );
    }

    #[test]
    fn references_of_the_fixture_cli() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet").canonicalize().unwrap();
        let refs = project_references(&root.join("src/Fixture.Cli/Fixture.Cli.csproj")).unwrap();
        assert_eq!(refs, [root.join("src/Fixture.Core/Fixture.Core.csproj")]);
    }

    /// Real SDK: the installed SDK's cache lists the console template. Needs `dotnet`.
    #[test]
    #[ignore]
    fn installed_templates() {
        let version = std::process::Command::new("dotnet").arg("--version").output().unwrap();
        let version = String::from_utf8_lossy(&version.stdout).trim().to_owned();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let text = std::fs::read_to_string(cache_file(&home, &version)).unwrap();
        let t = parse_cache(&text).unwrap();
        let console = t.iter().find(|t| t.short_name == "console").unwrap();
        assert!(console.parameters.iter().any(|p| p.option == "--framework"), "{console:?}");
        assert!(t.len() > 20);
    }
}
