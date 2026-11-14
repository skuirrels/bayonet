//! CI-10: "Show IL" from a file Roslyn wrote for a referenced assembly and from the
//! project's own code, through the .NET helper.

use ide_core::{IpcPath, NullSink, Pos};
use ide_host::{Host, HostConfig, ops};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn newest(dir: PathBuf) -> PathBuf {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    entries.pop().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the .NET 10 SDK and NuGet access"]
async fn shows_the_il_of_library_and_project_types() {
    let temp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp.path()).unwrap();
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("App.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><RollForward>LatestMajor</RollForward></PropertyGroup></Project>"#,
    )
    .unwrap();
    let source = "namespace Demo;\n\npublic class Greeter<T>\n{\n    public string Greet(T who) => $\"Hello, {who}\";\n\n    public struct Options { public int Times; }\n}\n";
    std::fs::write(app.join("Greeter.cs"), source).unwrap();

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
    host.trust.decide(&root, true, false).unwrap();
    host.open_project("main", &root).unwrap();
    let greeter = IpcPath(app.join("Greeter.cs"));

    // Not built yet: there is nothing to disassemble.
    let unbuilt = ops::il::dotnet_il_uri(&host, "main", greeter.clone(), None).await.unwrap_err();
    assert!(unbuilt.message.contains("App.csproj has not been built"), "{}", unbuilt.message);

    let built = std::process::Command::new("dotnet")
        .args(["build", "-nologo", "-v", "q"])
        .current_dir(&app)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stdout));

    // The caret in `Greet`: the type around it.
    let uri = ops::il::dotnet_il_uri(&host, "main", greeter.clone(), Some(Pos::new(4, 20))).await.unwrap();
    assert!(uri.starts_with("dotnet-il://App.dll/Greeter%601.il?"), "{uri}");
    let own = ops::lsp::lsp_virtual_document(&host, "main", uri.clone()).await.unwrap();
    assert_eq!(own.title, "Greeter`1.il");
    assert!(Path::new(own.description.as_deref().unwrap()).ends_with("App.dll"), "{:?}", own.description);
    assert!(own.text.contains("beforefieldinit Demo.Greeter`1<T>"), "{}", own.text);
    let greet = own.text.split("instance string Greet (").nth(1).unwrap();
    assert!(greet.contains("IL_0000:") && greet.contains("ret"), "{greet}");
    // The same document again is the one just made.
    assert_eq!(ops::lsp::lsp_virtual_document(&host, "main", uri).await.unwrap(), own);

    // The caret in the nested struct.
    let nested = ops::il::dotnet_il_uri(&host, "main", greeter, Some(Pos::new(6, 30))).await.unwrap();
    let nested = ops::lsp::lsp_virtual_document(&host, "main", nested).await.unwrap();
    assert_eq!(nested.title, "Greeter`1+Options.il");
    assert!(nested.text.contains("Times"), "{}", nested.text);

    // A decompiled framework type, as Go to Definition leaves it: the header names the
    // reference assembly, the IL comes from the shared runtime.
    let dotnet = dunce::canonicalize(ide_dotnet::helper::dotnet().unwrap()).unwrap();
    let reference = newest(newest(dotnet.parent().unwrap().join("packs/Microsoft.NETCore.App.Ref")).join("ref"))
        .join("System.Collections.dll");
    assert!(reference.is_file(), "{}", reference.display());
    let library = root.join("MetadataAsSource/b17c/DecompilationMetadataAsSourceFileProvider/020f/List.cs");
    std::fs::create_dir_all(library.parent().unwrap()).unwrap();
    std::fs::write(
        &library,
        format!(
            "\u{feff}#region Assembly System.Collections, Version=10.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a\n// {}\n// Decompiled with ICSharpCode.Decompiler \n#endregion\n\nnamespace System.Collections.Generic;\n\npublic class List<T>\n{{\n    public void Add(T item) {{ }}\n}}\n",
            reference.display()
        ),
    )
    .unwrap();
    let uri = ops::il::dotnet_il_uri(&host, "main", IpcPath(library), None).await.unwrap();
    assert!(uri.starts_with("dotnet-il://System.Collections.dll/List%601.il?"), "{uri}");
    let list = ops::lsp::lsp_virtual_document(&host, "main", uri).await.unwrap();
    assert_eq!(list.title, "List`1.il");
    let assembly = PathBuf::from(list.description.clone().unwrap());
    assert_eq!(assembly.file_name().unwrap(), "System.Private.CoreLib.dll");
    assert!(assembly.starts_with(dotnet.parent().unwrap().join("shared")), "{}", assembly.display());
    let add = list.text.split("instance void Add (").nth(1).unwrap();
    assert!(add.contains("IL_0000: ldarg.0"), "{add}");
    assert!(!list.text.contains("A reference assembly"));
    // The editor's buffer of it is read-only text.
    let buffer = ops::doc::doc_open_virtual(&host, "main", list.uri.clone()).await.unwrap();
    assert!(buffer.read_only && buffer.uri.as_deref() == Some(list.uri.as_str()));
    assert_eq!(buffer.text, list.text);

    // The library's own source (Source Link) has no header to read the assembly from.
    let sourced = root.join("MetadataAsSource/b17c/PdbSourceDocumentMetadataAsSourceFileProvider/11aa/Thing.cs");
    std::fs::create_dir_all(sourced.parent().unwrap()).unwrap();
    std::fs::write(&sourced, "namespace Lib;\npublic class Thing { }\n").unwrap();
    let unknown = ops::il::dotnet_il_uri(&host, "main", IpcPath(sourced), None).await.unwrap_err();
    assert!(unknown.message.contains("does not name the assembly"), "{}", unknown.message);
}
