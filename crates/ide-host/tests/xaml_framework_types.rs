//! CI-16: the Avalonia fixture's XAML can name its framework's elements, attributes and values.
//!
//! `#[ignore]`d: needs the .NET SDK and NuGet access.
//! `cargo test -p ide-host --test xaml_framework_types -- --ignored`

use ide_core::NullSink;
use ide_host::{Host, HostConfig, ops};
use std::path::Path;
use std::sync::Arc;

const WINDOW: &str = "main";

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if name == "bin" || name == "obj" || name == "TestResults" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() { copy(&src, &dst) } else { std::fs::copy(&src, &dst).map(|_| ()).unwrap() }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and NuGet access"]
async fn avalonia_xaml_names_its_framework_types() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap().join("avalonia");
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/avalonia"), &root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "AvaloniaFixture/AvaloniaFixture.csproj"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    let app = tmp.path().join("app");
    let host = Host::new(HostConfig {
        config_dir: app.join("config"),
        data_dir: app.join("data"),
        local_data_dir: app.join("data"),
        languages: Arc::new(ide_language::LanguageRegistry::load(None).unwrap()),
        syntax: Arc::default(),
        sink_for: Box::new(|_| Arc::new(NullSink)),
        owner: |w| w,
        runtime: tokio::runtime::Handle::current(),
        project_environment: false,
    });
    host.trust.decide(&root, true, false).unwrap();
    host.open_project(WINDOW, &root).unwrap();

    let xaml = root.join("AvaloniaFixture/MainWindow.axaml");
    let types = ops::xaml::dotnet_xaml_types(&host, WINDOW, xaml.clone().into()).await.unwrap();
    assert!(types.namespaces.iter().any(|m| m.xmlns == "https://github.com/avaloniaui"));
    let window = types.types.iter().find(|t| t.namespace == "Avalonia.Controls" && t.name == "Window").unwrap();
    assert!(window.base_type.is_some(), "{window:?}");
    assert!(types.types.iter().any(|t| t.name == "StackPanel"));
    // The second ask is served from the cache.
    let again = ops::xaml::dotnet_xaml_types(&host, WINDOW, xaml.into()).await.unwrap();
    assert_eq!(again.types.len(), types.types.len());
}
