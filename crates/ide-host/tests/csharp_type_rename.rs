//! CI-5: renaming a C# type through Roslyn also renames the file named after it.
//!
//! `#[ignore]`d: needs the .NET SDK and `roslyn-language-server` on `PATH`.
//! `cargo test -p ide-host --test csharp_type_rename -- --ignored`

use ide_core::{FileEdit, NullSink, Pos, RequestId, WorkspaceEdit};
use ide_host::{Host, HostConfig, ops};
use ide_lsp::types::Renamed;
use ide_workspace::FileMove;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Renames `word` on the first line of `file` that has it, once Roslyn has loaded the solution.
async fn rename(host: &Host, root: &Path, file: &str, word: &str, to: &str) -> Renamed {
    let info = ops::doc::open(host, WINDOW, root.join(file)).await.unwrap();
    let text = ops::doc::snapshot(host, WINDOW, info.id).await.unwrap().text;
    let (line, col) = text
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.find(word).map(|c| (i as u32, l[..c].encode_utf16().count() as u32)))
        .unwrap_or_else(|| panic!("{file} has no {word}"));
    let pos = Pos::new(line, col);
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        let prepared = ops::lsp::lsp_prepare_rename(host, WINDOW, info.id, pos).await;
        if matches!(prepared, Ok(Some(_)))
            && let Ok(renamed) = ops::lsp::lsp_rename(host, WINDOW, RequestId(7), info.id, pos, to.into()).await
            && renamed.edit.files.len() > 1
        {
            return renamed;
        }
        assert!(Instant::now() < deadline, "Roslyn did not rename {word}: {prepared:?}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn a_type_takes_its_file_along_and_a_member_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap().join("dotnet");
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet"), &root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
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

    // A member rename edits the file named after its type and leaves it where it is.
    let member = rename(&host, &root, "src/Fixture.Cli/Program.cs", "Greet(", "Salute").await;
    assert!(member.edit.files.iter().all(|f| matches!(f, FileEdit::Edit { .. })), "{:?}", member.edit);

    // From a usage in another project: the declaration is in src/Fixture.Core/Greeter.cs.
    let renamed = rename(&host, &root, "src/Fixture.Cli/Program.cs", "Greeter()", "Welcomer").await;
    let from = root.join("src/Fixture.Core/Greeter.cs");
    let to = root.join("src/Fixture.Core/Welcomer.cs");
    let moves: Vec<FileMove> = renamed
        .edit
        .files
        .iter()
        .filter_map(|f| match f {
            FileEdit::Rename { from, to } => Some(FileMove { from: from.clone(), to: to.clone() }),
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 1, "{:?}", renamed.edit);
    assert_eq!((&*moves[0].from, &*moves[0].to), (from.as_path(), to.as_path()));
    assert!(matches!(renamed.edit.files.last(), Some(FileEdit::Rename { .. })), "the move comes after the edits");

    // As the editor applies it: the text edits, then the move.
    let mut text = renamed.edit;
    text.files.retain(|f| matches!(f, FileEdit::Edit { .. }));
    ops::lsp::apply_workspace_edit(&host, WINDOW, text).await.unwrap();
    ops::refactor::refactor_move_files(&host, WINDOW, moves, WorkspaceEdit::default(), "Rename".into()).await.unwrap();
    assert!(!from.exists());
    let moved = std::fs::read_to_string(&to).unwrap();
    assert!(moved.contains("public sealed class Welcomer : IGreeter"), "{moved}");
    let program = std::fs::read_to_string(root.join("src/Fixture.Cli/Program.cs")).unwrap();
    assert!(program.contains("new Welcomer()"), "{program}");
}
