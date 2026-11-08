//! A project's Dependencies in the Solution Explorer (SLN-1): its shared frameworks,
//! NuGet packages (with the packages they bring in) and project references, per
//! target framework.
//!
//! Read from restore's `project.assets.json`. Before the first restore, or when that
//! file cannot be read, from the project file alone: packages then carry the version
//! as written (none under central package management) and nothing transitive.

use crate::project::read_project;
use ide_core::{IdeError, IdeResult, IpcPath};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDependencies {
    /// Read from `project.assets.json`; `false` = from the project file, before a restore.
    pub restored: bool,
    /// One per target framework, in the project's order.
    pub frameworks: Vec<FrameworkDependencies>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkDependencies {
    /// The target framework as the project names it (`net10.0`); empty when it names none.
    pub framework: String,
    /// Shared frameworks: `Microsoft.NETCore.App` first, then `Microsoft.AspNetCore.App`…
    pub framework_references: Vec<String>,
    /// Packages the project references first, then the ones they bring in, each by id.
    pub packages: Vec<PackageDependency>,
    /// Referenced project files, by name.
    pub projects: Vec<IpcPath>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackageDependency {
    pub id: String,
    /// The version restore picked, else the version or range as written.
    pub version: Option<String>,
    /// Referenced by the project itself, not only brought in by another package.
    pub top_level: bool,
    /// Ids (as in `packages`) of the packages this one depends on.
    pub dependencies: Vec<String>,
}

const NETCORE_APP: &str = "Microsoft.NETCore.App";

pub fn read_dependencies(project: &Path) -> IdeResult<ProjectDependencies> {
    if let Some(text) = assets_file(project).and_then(|p| std::fs::read_to_string(p).ok()) {
        match parse_assets(&text) {
            Ok(deps) => return Ok(deps),
            Err(e) => tracing::debug!("{}: unreadable project.assets.json: {e}", project.display()),
        }
    }
    from_project_file(project)
}

/// `obj/project.assets.json`, or `artifacts/obj/<name>/` under an ancestor (`UseArtifactsOutput`).
pub(crate) fn assets_file(project: &Path) -> Option<PathBuf> {
    const NAME: &str = "project.assets.json";
    let dir = project.parent()?;
    let local = dir.join("obj").join(NAME);
    if local.is_file() {
        return Some(local);
    }
    let name = project.file_stem()?;
    dir.ancestors().map(|a| a.join("artifacts").join("obj").join(name).join(NAME)).find(|p| p.is_file())
}

pub(crate) fn parse_assets(text: &str) -> IdeResult<ProjectDependencies> {
    let assets: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| IdeError::invalid(format!("not a valid project.assets.json: {e}")))?;
    let frameworks = assets["project"]["frameworks"]
        .as_object()
        .ok_or_else(|| IdeError::invalid("project.assets.json lists no frameworks"))?;
    let targets = assets["targets"].as_object();
    let restore = &assets["project"]["restore"]["frameworks"];
    let frameworks = frameworks
        .iter()
        .map(|(alias, spec)| FrameworkDependencies {
            framework: alias.clone(),
            framework_references: framework_references(alias, spec["frameworkReferences"].as_object()),
            packages: packages(spec, targets.and_then(|t| target_of(t, alias, frameworks.len()))),
            projects: sorted_projects(
                restore[alias]["projectReferences"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(key, r)| PathBuf::from(r["projectPath"].as_str().unwrap_or(key))),
            ),
        })
        .collect::<Vec<_>>();
    Ok(ProjectDependencies { restored: true, frameworks: in_project_order(frameworks, &assets) })
}

/// The project's order (`originalTargetFrameworks`): JSON objects come back sorted by key.
fn in_project_order(mut frameworks: Vec<FrameworkDependencies>, assets: &Value) -> Vec<FrameworkDependencies> {
    let order: Vec<&str> = assets["project"]["restore"]["originalTargetFrameworks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    frameworks.sort_by_key(|f| order.iter().position(|o| o.eq_ignore_ascii_case(&f.framework)).unwrap_or(usize::MAX));
    frameworks
}

/// The restore graph of `alias`: keyed by the alias (current SDKs) or by its long name.
fn target_of<'a>(targets: &'a Map<String, Value>, alias: &str, frameworks: usize) -> Option<&'a Map<String, Value>> {
    let rid_free = || targets.iter().filter(|(k, _)| !k.contains('/'));
    targets
        .get(alias)
        .or_else(|| long_name(alias).and_then(|n| targets.get(&n)))
        .or_else(|| (frameworks == 1).then(|| rid_free().map(|(_, v)| v).next()).flatten())
        .and_then(Value::as_object)
}

/// `net8.0` → `.NETCoreApp,Version=v8.0`, `netstandard2.0`, `net48` → `.NETFramework,Version=v4.8`.
fn long_name(alias: &str) -> Option<String> {
    let alias = alias.split('-').next().unwrap_or(alias);
    let (family, version) = if let Some(v) = alias.strip_prefix("netcoreapp") {
        (".NETCoreApp", v.to_owned())
    } else if let Some(v) = alias.strip_prefix("netstandard") {
        (".NETStandard", v.to_owned())
    } else {
        let v = alias.strip_prefix("net")?;
        if v.contains('.') {
            (".NETCoreApp", v.to_owned())
        } else if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
            (".NETFramework", v.chars().map(String::from).collect::<Vec<_>>().join("."))
        } else {
            return None;
        }
    };
    Some(format!("{family},Version=v{version}"))
}

/// `net5.0` and later, `netcoreapp*`: frameworks that run on `Microsoft.NETCore.App`.
fn is_netcore(alias: &str) -> bool {
    long_name(alias).is_some_and(|n| n.starts_with(".NETCoreApp"))
}

fn framework_references(alias: &str, declared: Option<&Map<String, Value>>) -> Vec<String> {
    let mut refs: Vec<String> = declared.into_iter().flatten().map(|(k, _)| k.clone()).collect();
    refs.sort_by_key(|r| r.to_lowercase());
    // Older SDKs leave the implicit base framework out of the assets file.
    let base = refs.iter().position(|r| r.eq_ignore_ascii_case(NETCORE_APP));
    match base {
        Some(i) => {
            let r = refs.remove(i);
            refs.insert(0, r);
        }
        None if is_netcore(alias) => refs.insert(0, NETCORE_APP.to_owned()),
        None => {}
    }
    refs
}

fn packages(spec: &Value, target: Option<&Map<String, Value>>) -> Vec<PackageDependency> {
    // Resolved packages by lower-case id: (id, version, dependency ids as written).
    let mut resolved: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    for (key, entry) in target.into_iter().flatten() {
        if entry["type"].as_str() != Some("package") {
            continue;
        }
        let (id, version) = key.split_once('/').unwrap_or((key, ""));
        let deps = entry["dependencies"].as_object().into_iter().flatten().map(|(d, _)| d.clone()).collect();
        resolved.insert(id.to_lowercase(), (id.to_owned(), version.to_owned(), deps));
    }
    let mut top: Vec<PackageDependency> = spec["dependencies"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, d)| d["target"].as_str().is_none_or(|t| t.eq_ignore_ascii_case("package")))
        .map(|(id, d)| match resolved.get(&id.to_lowercase()) {
            Some((id, version, _)) => package(id, version, true, &resolved),
            None => PackageDependency {
                id: id.clone(),
                version: d["version"].as_str().map(str::to_owned),
                top_level: true,
                dependencies: Vec::new(),
            },
        })
        .collect();
    let mut rest: Vec<PackageDependency> = resolved
        .iter()
        .filter(|(lower, _)| !top.iter().any(|t| t.id.to_lowercase() == **lower))
        .map(|(_, (id, version, _))| package(id, version, false, &resolved))
        .collect();
    top.sort_by_key(|p| p.id.to_lowercase());
    rest.sort_by_key(|p| p.id.to_lowercase());
    top.extend(rest);
    top
}

fn package(
    id: &str,
    version: &str,
    top_level: bool,
    resolved: &HashMap<String, (String, String, Vec<String>)>,
) -> PackageDependency {
    let deps = resolved.get(&id.to_lowercase()).map(|(_, _, d)| d.as_slice()).unwrap_or_default();
    let mut dependencies: Vec<String> =
        deps.iter().filter_map(|d| resolved.get(&d.to_lowercase()).map(|(id, _, _)| id.clone())).collect();
    dependencies.sort_by_key(|d| d.to_lowercase());
    PackageDependency {
        id: id.to_owned(),
        version: (!version.is_empty()).then(|| version.to_owned()),
        top_level,
        dependencies,
    }
}

fn sorted_projects(paths: impl Iterator<Item = PathBuf>) -> Vec<IpcPath> {
    let mut paths: Vec<PathBuf> = paths.collect();
    paths.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    paths.dedup();
    paths.into_iter().map(IpcPath::from).collect()
}

fn from_project_file(project: &Path) -> IdeResult<ProjectDependencies> {
    let info = read_project(project)?;
    let dir = project.parent().unwrap_or(project);
    let web = info.sdks.iter().any(|s| s.starts_with("Microsoft.NET.Sdk.Web") || s == "Microsoft.NET.Sdk.Razor");
    let mut packages: Vec<PackageDependency> = info
        .package_references
        .iter()
        .map(|(id, version)| PackageDependency {
            id: id.clone(),
            version: version.clone(),
            top_level: true,
            dependencies: Vec::new(),
        })
        .collect();
    packages.sort_by_key(|p| p.id.to_lowercase());
    let projects = sorted_projects(info.project_references.iter().map(|r| {
        let joined = dir.join(r.replace('\\', "/"));
        std::fs::canonicalize(&joined).unwrap_or(joined)
    }));
    let aliases = if info.target_frameworks.is_empty() { vec![String::new()] } else { info.target_frameworks };
    let frameworks = aliases
        .into_iter()
        .map(|alias| {
            let mut framework_references = Vec::new();
            if is_netcore(&alias) {
                framework_references.push(NETCORE_APP.to_owned());
                if web {
                    framework_references.push("Microsoft.AspNetCore.App".to_owned());
                }
            }
            FrameworkDependencies {
                framework: alias,
                framework_references,
                packages: packages.clone(),
                projects: projects.clone(),
            }
        })
        .collect();
    Ok(ProjectDependencies { restored: false, frameworks })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSETS: &str = r#"{
      "version": 3,
      "targets": {
        "net10.0": {
          "Microsoft.AspNetCore.OpenApi/10.0.14": { "type": "package", "dependencies": { "Microsoft.OpenApi": "[2.12.2, 3.0.0)" } },
          "Microsoft.OpenApi/2.12.2": { "type": "package" },
          "Fixture.Core/1.0.0": { "type": "project", "framework": ".NETCoreApp,Version=v10.0" }
        },
        "net10.0/osx-arm64": {}
      },
      "project": {
        "restore": {
          "frameworks": {
            "net10.0": {
              "projectReferences": {
                "/s/src/Fixture.Core/Fixture.Core.csproj": { "projectPath": "/s/src/Fixture.Core/Fixture.Core.csproj" },
                "/s/src/Analyzers/Analyzers.csproj": { "projectPath": "/s/src/Analyzers/Analyzers.csproj" }
              }
            }
          }
        },
        "frameworks": {
          "net10.0": {
            "dependencies": {
              "Microsoft.AspNetCore.OpenApi": { "target": "Package", "version": "[10.0.14, )" },
              "Unreachable.Package": { "target": "Package", "version": "[1.2.3, )" }
            },
            "frameworkReferences": { "Microsoft.NETCore.App": {}, "Microsoft.AspNetCore.App": {} }
          }
        }
      }
    }"#;

    #[test]
    fn reads_packages_projects_and_frameworks_from_assets() {
        let deps = parse_assets(ASSETS).unwrap();
        assert!(deps.restored);
        let [fw] = deps.frameworks.as_slice() else { panic!("one framework") };
        assert_eq!(fw.framework, "net10.0");
        assert_eq!(fw.framework_references, ["Microsoft.NETCore.App", "Microsoft.AspNetCore.App"]);
        assert_eq!(
            fw.projects,
            [
                IpcPath::from(PathBuf::from("/s/src/Analyzers/Analyzers.csproj")),
                IpcPath::from(PathBuf::from("/s/src/Fixture.Core/Fixture.Core.csproj"))
            ]
        );
        assert_eq!(
            fw.packages,
            [
                PackageDependency {
                    id: "Microsoft.AspNetCore.OpenApi".into(),
                    version: Some("10.0.14".into()),
                    top_level: true,
                    dependencies: vec!["Microsoft.OpenApi".into()],
                },
                // Not resolved (its feed was unreachable): the range as written.
                PackageDependency {
                    id: "Unreachable.Package".into(),
                    version: Some("[1.2.3, )".into()),
                    top_level: true,
                    dependencies: vec![],
                },
                PackageDependency {
                    id: "Microsoft.OpenApi".into(),
                    version: Some("2.12.2".into()),
                    top_level: false,
                    dependencies: vec![],
                },
            ]
        );
    }

    #[test]
    fn finds_targets_by_long_name_and_adds_the_implicit_base_framework() {
        let text = r#"{
          "targets": {
            ".NETCoreApp,Version=v8.0": { "Newtonsoft.Json/13.0.3": { "type": "package" } },
            ".NETFramework,Version=v4.8": { "Newtonsoft.Json/13.0.1": { "type": "package" } }
          },
          "project": { "restore": { "originalTargetFrameworks": ["net8.0", "net48"] }, "frameworks": {
            "net8.0": { "dependencies": { "Newtonsoft.Json": { "target": "Package", "version": "[13.0.1, )" } } },
            "net48": { "dependencies": { "Newtonsoft.Json": { "target": "Package", "version": "[13.0.1, )" } } }
          } }
        }"#;
        let deps = parse_assets(text).unwrap();
        let versions: Vec<_> =
            deps.frameworks.iter().map(|f| (f.framework.as_str(), f.packages[0].version.as_deref())).collect();
        assert_eq!(versions, [("net8.0", Some("13.0.3")), ("net48", Some("13.0.1"))]);
        assert_eq!(deps.frameworks[0].framework_references, [NETCORE_APP]);
        assert!(deps.frameworks[1].framework_references.is_empty());
    }

    #[test]
    fn names_frameworks() {
        assert_eq!(long_name("net10.0").as_deref(), Some(".NETCoreApp,Version=v10.0"));
        assert_eq!(long_name("net8.0-windows").as_deref(), Some(".NETCoreApp,Version=v8.0"));
        assert_eq!(long_name("netcoreapp3.1").as_deref(), Some(".NETCoreApp,Version=v3.1"));
        assert_eq!(long_name("netstandard2.0").as_deref(), Some(".NETStandard,Version=v2.0"));
        assert_eq!(long_name("net472").as_deref(), Some(".NETFramework,Version=v4.7.2"));
        assert_eq!(long_name("uap10.0"), None);
    }

    #[test]
    fn falls_back_to_the_project_file_before_a_restore() {
        let dir = tempfile::tempdir().unwrap();
        let core = dir.path().join("Core");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::write(core.join("Core.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
        let app = dir.path().join("App");
        std::fs::create_dir_all(&app).unwrap();
        let project = app.join("App.csproj");
        std::fs::write(
            &project,
            r#"<Project Sdk="Microsoft.NET.Sdk.Web">
                 <PropertyGroup><TargetFrameworks>net10.0;net48</TargetFrameworks></PropertyGroup>
                 <ItemGroup>
                   <PackageReference Include="Serilog" Version="4.0.0" />
                   <PackageReference Include="Dapper" />
                   <ProjectReference Include="..\Core\Core.csproj" />
                 </ItemGroup>
               </Project>"#,
        )
        .unwrap();
        let deps = read_dependencies(&project).unwrap();
        assert!(!deps.restored);
        let [modern, framework] = deps.frameworks.as_slice() else { panic!("two frameworks") };
        assert_eq!(modern.framework, "net10.0");
        assert_eq!(modern.framework_references, [NETCORE_APP, "Microsoft.AspNetCore.App"]);
        assert!(framework.framework_references.is_empty());
        let packages: Vec<_> = modern.packages.iter().map(|p| (p.id.as_str(), p.version.as_deref())).collect();
        assert_eq!(packages, [("Dapper", None), ("Serilog", Some("4.0.0"))]);
        let core = std::fs::canonicalize(core.join("Core.csproj")).unwrap();
        assert_eq!(modern.projects, [IpcPath::from(core)]);

        // A restore's assets file wins.
        std::fs::create_dir_all(app.join("obj")).unwrap();
        std::fs::write(app.join("obj/project.assets.json"), ASSETS).unwrap();
        assert!(read_dependencies(&project).unwrap().restored);
    }
}
