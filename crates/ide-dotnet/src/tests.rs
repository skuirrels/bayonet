use super::*;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet")
}

/// `folder/` for folders, the project name for projects, `file:name` for items.
fn outline(items: &[SolutionItem], indent: usize, out: &mut Vec<String>) {
    for item in items {
        let pad = "  ".repeat(indent);
        match item {
            SolutionItem::Folder { name, items } => {
                out.push(format!("{pad}{name}/"));
                outline(items, indent + 1, out);
            }
            SolutionItem::Project { name, .. } => out.push(format!("{pad}{name}")),
            SolutionItem::File { path } => {
                out.push(format!("{pad}file:{}", path.file_name().unwrap().to_string_lossy()))
            }
        }
    }
}

fn lines(s: &Solution) -> Vec<String> {
    let mut out = Vec::new();
    outline(&s.items, 0, &mut out);
    out
}

const FIXTURE_TREE: [&str; 9] = [
    "src/",
    "  Fixture.Api",
    "  Fixture.Cli",
    "  Fixture.Core",
    "  Fixture.FSharp",
    "tests/",
    "  Fixture.Core.MSTests",
    "  Fixture.Core.NUnitTests",
    "  Fixture.Core.XunitTests",
];

#[test]
fn fixture_slnx_and_sln_describe_the_same_tree() {
    let slnx = read(&fixture().join("DotnetFixture.slnx")).unwrap();
    let sln = read(&fixture().join("DotnetFixture.sln")).unwrap();
    assert_eq!(slnx.name, "DotnetFixture");
    assert_eq!(slnx.format, SolutionFormat::Slnx);
    assert_eq!(sln.format, SolutionFormat::Sln);
    assert_eq!(lines(&slnx), FIXTURE_TREE);
    assert_eq!(lines(&sln), FIXTURE_TREE);
    assert_eq!(slnx.items, sln.items);
}

#[test]
fn projects_resolve_to_existing_files_with_their_language() {
    let s = read(&fixture().join("DotnetFixture.sln")).unwrap();
    let SolutionItem::Folder { items, .. } = &s.items[0] else { panic!() };
    let SolutionItem::Project { path, language, missing, .. } = &items[3] else { panic!() };
    assert_eq!(*language, ProjectLanguage::FSharp);
    assert!(!missing);
    assert!(path.ends_with("src/Fixture.FSharp/Fixture.FSharp.fsproj"), "{path:?}");
    assert!(path.is_absolute() || fixture().is_relative());
}

#[test]
fn slnx_nests_folders_by_path_and_keeps_solution_items() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("App.slnx");
    std::fs::write(
        &path,
        r#"<Solution>
  <Folder Name="/Solution Items/">
    <File Path="README.md" />
  </Folder>
  <Folder Name="/src/libs/">
    <Project Path="src\libs\Lib\Lib.vbproj" />
  </Folder>
  <Folder Name="/src/">
    <Project Path="src/App/App.csproj" Type="Classic C#" />
  </Folder>
  <Project Path="../outside/Tool.csproj" />
</Solution>"#,
    )
    .unwrap();
    let s = read(&path).unwrap();
    assert_eq!(lines(&s), ["Solution Items/", "  file:README.md", "src/", "  libs/", "    Lib", "  App", "Tool"]);
    let SolutionItem::Project { path: tool, missing, .. } = &s.items[2] else { panic!() };
    assert_eq!(tool.as_ref() as &Path, dir.path().parent().unwrap().join("outside/Tool.csproj"));
    assert!(missing);
}

#[test]
fn sln_nests_folders_through_nested_projects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("App.sln");
    std::fs::write(
        &path,
        "\u{feff}
Microsoft Visual Studio Solution File, Format Version 12.00
Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"Solution Items\", \"Solution Items\", \"{AAAA0000-0000-0000-0000-000000000001}\"
	ProjectSection(SolutionItems) = preProject
		Directory.Build.props = Directory.Build.props
	EndProjectSection
EndProject
Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"src\", \"src\", \"{aaaa0000-0000-0000-0000-000000000002}\"
EndProject
Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"inner\", \"inner\", \"{AAAA0000-0000-0000-0000-000000000003}\"
EndProject
Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"App\", \"src\\App\\App.csproj\", \"{BBBB0000-0000-0000-0000-000000000001}\"
EndProject
Project(\"{F2A71F9B-5D33-465A-A702-920D77279786}\") = \"Fun\", \"src\\inner\\Fun\\Fun.fsproj\", \"{BBBB0000-0000-0000-0000-000000000002}\"
EndProject
Global
	GlobalSection(NestedProjects) = preSolution
		{AAAA0000-0000-0000-0000-000000000003} = {AAAA0000-0000-0000-0000-000000000002}
		{BBBB0000-0000-0000-0000-000000000001} = {AAAA0000-0000-0000-0000-000000000002}
		{BBBB0000-0000-0000-0000-000000000002} = {AAAA0000-0000-0000-0000-000000000003}
	EndGlobalSection
EndGlobal
",
    )
    .unwrap();
    let s = read(&path).unwrap();
    assert_eq!(lines(&s), ["Solution Items/", "  file:Directory.Build.props", "src/", "  inner/", "    Fun", "  App"]);
}

#[test]
fn slnf_keeps_the_filtered_projects_and_their_folders() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("DotnetFixture.slnx");
    std::fs::copy(fixture().join("DotnetFixture.slnx"), &copy).unwrap();
    let filter = dir.path().join("Core.slnf");
    std::fs::write(
        &filter,
        r#"{ "solution": { "path": "DotnetFixture.slnx", "projects": ["src\\Fixture.Core\\Fixture.Core.csproj", "src/Fixture.Cli/Fixture.Cli.csproj"] } }"#,
    )
    .unwrap();
    let s = read(&filter).unwrap();
    assert_eq!(s.format, SolutionFormat::Slnf);
    assert_eq!(s.name, "Core");
    assert_eq!(lines(&s), ["src/", "  Fixture.Cli", "  Fixture.Core"]);
}

#[test]
fn find_prefers_slnx_then_sln() {
    let mut found = find(&fixture());
    found.iter_mut().for_each(|p| *p = p.file_name().unwrap().into());
    assert_eq!(found, [PathBuf::from("DotnetFixture.slnx"), PathBuf::from("DotnetFixture.sln")]);
}

#[test]
fn malformed_files_are_errors_not_empty_solutions() {
    let dir = tempfile::tempdir().unwrap();
    let sln = dir.path().join("Bad.sln");
    std::fs::write(&sln, "not a solution").unwrap();
    assert!(read(&sln).is_err());
    let slnx = dir.path().join("Bad.slnx");
    std::fs::write(&slnx, "<Solution><Folder Name=\"/a/\"></Solution>").unwrap();
    assert!(read(&slnx).is_err());
}
