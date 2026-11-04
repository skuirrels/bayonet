//! CI-16: the C# types XAML bindings resolve against, found through the index.

use ide_core::NullSink;
use ide_host::{Host, HostConfig, ops};
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_model_members_merge_partials_and_follow_base_types() {
    let temp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp.path()).unwrap();
    let vm = root.join("ViewModels");
    std::fs::create_dir_all(&vm).unwrap();
    std::fs::write(
        vm.join("MainViewModel.cs"),
        "namespace App.ViewModels;\npublic partial class MainViewModel : ViewModelBase\n{\n    [ObservableProperty] private string _name = \"\";\n    public User Owner { get; set; } = new();\n}\n",
    )
    .unwrap();
    std::fs::write(
        vm.join("MainViewModel.Commands.cs"),
        "namespace App.ViewModels;\npublic partial class MainViewModel\n{\n    [RelayCommand] private void Save() { }\n}\n",
    )
    .unwrap();
    std::fs::write(
        vm.join("ViewModelBase.cs"),
        "namespace App.ViewModels;\npublic abstract class ViewModelBase : ObservableObject\n{\n    public bool IsBusy { get; set; }\n    public string Name { get; set; } = \"\";\n}\n",
    )
    .unwrap();
    // The same simple name in another namespace.
    std::fs::write(
        root.join("Other.cs"),
        "namespace Tools;\npublic class MainViewModel { public int Elsewhere { get; } }\n",
    )
    .unwrap();
    let host = Host::new(HostConfig {
        config_dir: root.join(".config"),
        data_dir: root.join(".data"),
        local_data_dir: root.join(".data"),
        languages: Arc::new(ide_language::LanguageRegistry::load(None).unwrap()),
        syntax: Arc::default(),
        sink_for: Box::new(|_| Arc::new(NullSink)),
        owner: |w| w,
        runtime: tokio::runtime::Handle::current(),
        project_environment: false,
    });
    let project = host.open_project("main", &root).unwrap();
    assert!(project.index.wait_idle(Duration::from_secs(20)));

    let t = ops::analysis::csharp_type_members(&host, "main", "MainViewModel".into(), Some("App.ViewModels".into()))
        .await
        .unwrap()
        .expect("the view model");
    let members: Vec<(&str, &str)> = t.members.iter().map(|m| (m.name.as_str(), m.declared_in.as_str())).collect();
    // Partial parts by path; `Name` from the generated property hides the base type's.
    assert_eq!(
        members,
        [
            ("SaveCommand", "MainViewModel"),
            ("Name", "MainViewModel"),
            ("Owner", "MainViewModel"),
            ("IsBusy", "ViewModelBase")
        ]
    );
    assert!(t.members[0].path.ends_with("MainViewModel.Commands.cs"));

    let other = ops::analysis::csharp_type_members(&host, "main", "MainViewModel".into(), Some("Tools".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(other.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Elsewhere"]);
    assert!(ops::analysis::csharp_type_members(&host, "main", "Missing".into(), None).await.unwrap().is_none());
}
