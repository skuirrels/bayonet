//! What the IDE needs from a project file before MSBuild has evaluated it: whether
//! it runs, whether it holds tests, and its target frameworks. Read from the file
//! itself (properties set in `Directory.Build.props` are not seen), so a project
//! is classified instantly and without an SDK.

use ide_core::{IdeError, IdeResult};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInfo {
    /// `Sdk="…"` of `<Project>` plus any `<Sdk Name="…"/>`, without versions.
    pub sdks: Vec<String>,
    pub output_type: Option<String>,
    /// `TargetFramework`, or the `;`-separated `TargetFrameworks`, in file order.
    pub target_frameworks: Vec<String>,
    /// A test project: `IsTestProject`, a test SDK or runner, or a test framework package.
    pub is_test: bool,
    /// `<ProjectReference Include="…"/>` values as written (relative, `\` or `/`).
    pub project_references: Vec<String>,
}

impl ProjectInfo {
    /// Can be started with `dotnet run`: an executable, or an SDK whose projects are.
    pub fn is_executable(&self) -> bool {
        if self.is_test {
            return false;
        }
        let exe = self
            .output_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("exe") || t.eq_ignore_ascii_case("winexe"));
        exe || self.sdks.iter().any(|s| {
            matches!(
                s.as_str(),
                "Microsoft.NET.Sdk.Web" | "Microsoft.NET.Sdk.Worker" | "Microsoft.NET.Sdk.BlazorWebAssembly"
            )
        })
    }
}

/// Packages that make a project a test project.
const TEST_PACKAGES: [&str; 8] = [
    "Microsoft.NET.Test.Sdk",
    "xunit",
    "xunit.v3",
    "NUnit",
    "MSTest",
    "MSTest.TestFramework",
    "TUnit",
    "Microsoft.Testing.Platform",
];

/// Properties that, when `true`, make a project a test project.
const TEST_PROPERTIES: [&str; 4] =
    ["IsTestProject", "EnableMSTestRunner", "EnableNUnitRunner", "UseMicrosoftTestingPlatformRunner"];

pub fn read_project(path: &Path) -> IdeResult<ProjectInfo> {
    parse_project(&std::fs::read_to_string(path)?)
}

pub(crate) fn parse_project(text: &str) -> IdeResult<ProjectInfo> {
    let invalid = |e: &dyn std::fmt::Display| IdeError::invalid(format!("not a valid project file: {e}"));
    let mut reader = Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut info = ProjectInfo::default();
    // The innermost open element, for the text inside it.
    let mut open: Option<String> = None;
    let mut target_framework: Option<String> = None;
    let mut target_frameworks: Option<String> = None;
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| invalid(&e))?;
        match &event {
            Event::Start(tag) | Event::Empty(tag) => {
                let name = tag.name().as_ref().to_owned();
                match name.as_str() {
                    "Project" => info.sdks.extend(attr(tag, "Sdk").iter().flat_map(|s| sdk_names(s))),
                    "Sdk" => info.sdks.extend(attr(tag, "Name").iter().flat_map(|s| sdk_names(s))),
                    "ProjectReference" => info.project_references.extend(attr(tag, "Include")),
                    "PackageReference"
                        if attr(tag, "Include")
                            .is_some_and(|p| TEST_PACKAGES.iter().any(|t| t.eq_ignore_ascii_case(&p))) =>
                    {
                        info.is_test = true;
                    }
                    _ => {}
                }
                if info.sdks.iter().any(|s| s == "MSTest.Sdk") {
                    info.is_test = true;
                }
                open = matches!(event, Event::Start(_)).then_some(name);
                depth += usize::from(open.is_some());
            }
            Event::Text(t) => {
                let Some(name) = &open else { continue };
                let value = t.xml_content(quick_xml::XmlVersion::Implicit1_0).trim().to_owned();
                match name.as_str() {
                    "OutputType" => info.output_type = Some(value),
                    "TargetFramework" => target_framework = Some(value),
                    "TargetFrameworks" => target_frameworks = Some(value),
                    n if TEST_PROPERTIES.contains(&n) && value.eq_ignore_ascii_case("true") => info.is_test = true,
                    _ => {}
                }
            }
            Event::End(_) => {
                open = None;
                depth = depth.saturating_sub(1);
            }
            Event::Eof if depth > 0 => return Err(invalid(&"unexpected end of file")),
            Event::Eof => break,
            _ => {}
        }
    }
    // `TargetFramework` wins when both are set, as in MSBuild's inner builds.
    info.target_frameworks = match (target_framework, target_frameworks) {
        (Some(one), _) if !one.is_empty() => vec![one],
        (_, Some(many)) => many.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    Ok(info)
}

fn attr(tag: &BytesStart, name: &str) -> Option<String> {
    tag.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// `Microsoft.NET.Sdk;Aspire.AppHost.Sdk/9.0.0` → the names.
fn sdk_names(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(|s| s.split('/').next().unwrap_or(s).trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A `Project` launch profile from `Properties/launchSettings.json`: how `dotnet run
/// --launch-profile` starts the project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchProfile {
    #[serde(skip)]
    pub name: String,
    pub command_name: String,
    pub command_line_args: Option<String>,
    pub working_directory: Option<String>,
    pub application_url: Option<String>,
    pub environment_variables: BTreeMap<String, String>,
}

/// The launch profiles `dotnet run` can use (`commandName: "Project"`), by name.
/// IIS Express, Docker and executable profiles are for other hosts and are left out.
pub fn launch_profiles(project_dir: &Path) -> Vec<LaunchProfile> {
    let Ok(text) = std::fs::read_to_string(project_dir.join("Properties/launchSettings.json")) else {
        return Vec::new();
    };
    parse_launch_profiles(&text)
}

pub(crate) fn parse_launch_profiles(text: &str) -> Vec<LaunchProfile> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        profiles: serde_json::Map<String, serde_json::Value>,
    }
    let Ok(file) = serde_json::from_str::<File>(text.trim_start_matches('\u{feff}')) else { return Vec::new() };
    file.profiles
        .into_iter()
        .filter_map(|(name, v)| {
            let mut profile: LaunchProfile = serde_json::from_value(v).ok()?;
            profile.name = name;
            (profile.command_name == "Project").then_some(profile)
        })
        .collect()
}
