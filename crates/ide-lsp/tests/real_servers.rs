//! Integration tests against real language servers. The server tests are
//! `#[ignore]`d; run with `cargo test -p ide-lsp --test real_servers -- --ignored`.

use ide_core::event::ServerStatus;
use ide_core::{DocId, IdeEvent, LanguageId, Pos, Range};
use ide_language::{LanguageConfig, LanguageRegistry, LanguageServerConfig};
use ide_lsp::LspManager;
use ide_lsp::types::GotoKind;
use ide_text::AppliedChange;
use parking_lot::Mutex;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Events = Arc<Mutex<Vec<IdeEvent>>>;

fn manager(root: &Path, registry: LanguageRegistry) -> (Arc<LspManager>, Events) {
    let events: Events = Arc::default();
    let sink = {
        let events = events.clone();
        Arc::new(move |e: IdeEvent| events.lock().push(e))
    };
    (LspManager::new(root.to_path_buf(), Arc::new(registry), sink), events)
}

fn last_status(events: &Events, name: &str) -> Option<(ServerStatus, Option<String>)> {
    events.lock().iter().rev().find_map(|e| match e {
        IdeEvent::LanguageServer { name: n, status, message, .. } if n == name => Some((*status, message.clone())),
        _ => None,
    })
}

/// Retries `f` until it yields `Some` (servers answer emptily while indexing).
async fn retry<T, F: Future<Output = Option<T>>>(what: &str, timeout: Duration, mut f: impl FnMut() -> F) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn lang(id: &str, ext: &str, server: LanguageServerConfig) -> LanguageConfig {
    LanguageConfig {
        id: LanguageId::new(id),
        name: id.into(),
        extensions: vec![ext.into()],
        filenames: vec![],
        first_line: None,
        line_comment: None,
        block_comment: None,
        indent: None,
        ui_mode: None,
        framework_ui_modes: Vec::new(),
        formatter: None,
        tree_sitter: None,
        language_servers: vec![server],
        debugger: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires jdtls on PATH and a compatible JDK"]
async fn intellij_java_formatter_preserves_manual_breaks_and_wraps_long_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = include_str!("fixtures/FormatterSample.input.java");
    let expected = include_str!("fixtures/FormatterSample.intellij.java");
    let path = root.join("src/demo/FormatterSample.java");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    let mut java = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("java")).unwrap();
    let server = &mut java.language_servers[0];
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_JDTLS") {
        server.command = bin;
    }
    server.args.extend(["-data".into(), root.join("jdtls-workspace").to_string_lossy().into_owned()]);
    server.settings = Some(serde_json::json!({"java": {"autobuild": {"enabled": false}}}));
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![java]));
    mgr.set_config_hook(Box::new({
        let root = root.clone();
        move |cfg, _| {
            ide_lsp::jdtls::configure_source_paths(cfg, Some(vec!["src".into()]));
            let style = ide_core::editor_settings::LanguageCodeStyle {
                formatter: Some("intellij".into()),
                ..Default::default()
            };
            ide_lsp::jdtls::configure_formatter(cfg, Some(&style), &root, &root.join("data"), None).unwrap();
        }
    }));
    let doc = DocId::next();
    mgr.did_open(doc, &path, &LanguageId::new("java"), 1, source);
    retry("jdtls ready", Duration::from_secs(120), || {
        let status = last_status(&events, "jdtls");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &status {
            panic!("jdtls failed: {msg:?}");
        }
        async move { matches!(status, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;
    let edits = mgr
        .format(
            doc,
            None,
            ide_lsp::types::FormatOptions { tab_size: 4, insert_spaces: true, on_save: false, tool: None },
        )
        .await;
    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
    let mut document = ide_text::Document::new(source);
    document.apply_edits(&edits.unwrap()).unwrap();
    assert_eq!(document.text(), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_binary_is_reported_with_install_hint() {
    let dir = tempfile::tempdir().unwrap();
    let server = LanguageServerConfig {
        name: "nope-ls".into(),
        command: "openidea-definitely-not-installed-ls".into(),
        args: vec![],
        env: Default::default(),
        root_markers: vec![],
        require_root_marker: false,
        initialization_options: None,
        settings: None,
        install_hint: Some("npm i -g nope-ls".into()),
        connect: None,
        fallback: false,
        features: Vec::new(),
        code_action_kinds: Vec::new(),
        max_java: None,
        launch_error: None,
        tsserver_project: false,
        pull_diagnostics: false,
        workspace_diagnostics: false,
        pinned: None,
    };
    let (mgr, events) = manager(dir.path(), LanguageRegistry::from_configs(vec![lang("nope", "nope", server)]));
    let path = dir.path().join("a.nope");
    mgr.did_open(DocId::next(), &path, &LanguageId::new("nope"), 1, "x");
    retry("missing status", Duration::from_secs(10), || {
        let s = last_status(&events, "nope-ls");
        async move { (s.as_ref().map(|s| s.0) == Some(ServerStatus::Missing)).then_some(()) }
    })
    .await;
    assert_eq!(last_status(&events, "nope-ls").unwrap().1.as_deref(), Some("npm i -g nope-ls"));
}

const MAIN_RS: &str = "struct Point {
    x: i32,
    y: i32,
}

/// Adds two numbers.
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let s = add(p.x, p.y);
    println!(\"{s}\");
}
";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires ty on PATH"]
async fn ty_python_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("example.py");
    let source = "def greet(name: str) -> str:\n    return name\n\nresult = greet('Ada')\nbad: int = 'wrong'\n";
    std::fs::write(&path, source).unwrap();
    let mut python = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("python")).unwrap();
    python.language_servers.retain(|server| server.name == "ty");
    assert_eq!(python.language_servers.len(), 1);
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![python]));
    let doc = DocId::next();
    mgr.did_open(doc, &path, &LanguageId::new("python"), 1, source);
    retry("ty ready", Duration::from_secs(30), || {
        let status = last_status(&events, "ty");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &status {
            panic!("ty failed: {msg:?}");
        }
        async move { matches!(status, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;
    let defs = retry("ty definition", Duration::from_secs(30), || {
        let mgr = mgr.clone();
        async move { mgr.goto(doc, Pos::new(3, 10), GotoKind::Definition).await.ok().filter(|d| !d.is_empty()) }
    })
    .await;
    assert_eq!(defs[0].path, path);
    assert_eq!(defs[0].range.start.line, 0);
    retry("ty diagnostics", Duration::from_secs(30), || {
        let found = events.lock().iter().any(
            |event| matches!(event, IdeEvent::Diagnostics { source, items, .. } if source == "ty" && !items.is_empty()),
        );
        async move { found.then_some(()) }
    })
    .await;
}

/// Ruff next to basedpyright in a project configured for Ruff: its lint
/// diagnostics, the only formatter and the only Organize Imports and Fix All.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires ruff and basedpyright on PATH"]
async fn ruff_next_to_basedpyright_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("pyproject.toml"), "[tool.ruff.lint]\nselect = [\"F\", \"I\"]\n").unwrap();
    let path = root.join("example.py");
    let source = "import sys\nimport os\n\nx  =  os.getcwd( )\n";
    std::fs::write(&path, source).unwrap();
    let mut python = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("python")).unwrap();
    python.language_servers.retain(|server| matches!(server.name.as_str(), "basedpyright" | "ruff"));
    assert_eq!(python.language_servers.len(), 2);
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![python]));
    let doc = DocId::next();
    mgr.did_open(doc, &path, &LanguageId::new("python"), 1, source);
    for name in ["basedpyright", "ruff"] {
        retry(name, Duration::from_secs(60), || {
            let status = last_status(&events, name);
            if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &status {
                panic!("{name} failed: {msg:?}");
            }
            async move { matches!(status, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
        })
        .await;
    }
    // F401 (`sys` unused) and I001 (unsorted imports).
    let codes = retry("ruff diagnostics", Duration::from_secs(30), || {
        let codes: Option<Vec<String>> = events.lock().iter().rev().find_map(|e| match e {
            IdeEvent::Diagnostics { source, items, .. } if source == "ruff" && !items.is_empty() => {
                Some(items.iter().filter_map(|d| d.code.clone()).collect())
            }
            _ => None,
        });
        async move { codes }
    })
    .await;
    assert!(codes.iter().any(|c| c == "F401") && codes.iter().any(|c| c == "I001"), "{codes:?}");

    let options = ide_lsp::types::FormatOptions { tab_size: 4, insert_spaces: true, on_save: false, tool: None };
    let edits = mgr.format(doc, None, options).await.unwrap();
    assert!(!edits.is_empty() && edits.iter().all(|e| e.range.start.line == 3), "{edits:?}");

    let whole = Range::new(Pos::new(0, 0), Pos::new(4, 0));
    for kind in ["source.organizeImports", "source.fixAll"] {
        let actions = mgr.code_actions(doc, whole, vec![kind.into()]).await.unwrap();
        let kinds: Vec<&str> = actions.iter().filter_map(|a| a.kind.as_deref()).collect();
        assert_eq!(kinds, [format!("{kind}.ruff")], "{actions:?}");
        let action = mgr.resolve_code_action(doc, actions[0].clone()).await.unwrap();
        let edit = action.edit.expect("an edit");
        assert!(!edited_lines(&edit, &path).is_empty(), "{edit:?}");
    }
    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires pyrefly on PATH"]
async fn pyrefly_python_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("example.py");
    let source = "def greet(name: str) -> str:\n    return name\n\nresult = greet('Ada')\nbad: int = 'wrong'\n";
    std::fs::write(&path, source).unwrap();
    // Pyrefly only reports type errors in projects with a Pyrefly configuration.
    std::fs::write(root.join("pyrefly.toml"), "").unwrap();
    let mut python = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("python")).unwrap();
    python.language_servers.retain(|server| server.name == "pyrefly");
    assert_eq!(python.language_servers.len(), 1);
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![python]));
    let doc = DocId::next();
    mgr.did_open(doc, &path, &LanguageId::new("python"), 1, source);
    retry("pyrefly ready", Duration::from_secs(30), || {
        let status = last_status(&events, "pyrefly");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &status {
            panic!("pyrefly failed: {msg:?}");
        }
        async move { matches!(status, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;
    let defs = retry("pyrefly definition", Duration::from_secs(30), || {
        let mgr = mgr.clone();
        async move { mgr.goto(doc, Pos::new(3, 10), GotoKind::Definition).await.ok().filter(|d| !d.is_empty()) }
    })
    .await;
    assert_eq!(defs[0].path, path);
    assert_eq!(defs[0].range.start.line, 0);
    retry("pyrefly diagnostics", Duration::from_secs(30), || {
        let found = events.lock().iter().any(
            |event| matches!(event, IdeEvent::Diagnostics { source, items, .. } if source == "pyrefly" && !items.is_empty()),
        );
        async move { found.then_some(()) }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires rust-analyzer"]
async fn rust_analyzer_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
        .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let main = root.join("src/main.rs");
    std::fs::write(&main, MAIN_RS).unwrap();

    // `OPENIDEA_TEST_RUST_ANALYZER` overrides the binary (e.g. when the rustup proxy
    // has no rust-analyzer component for the default toolchain).
    let mut rust = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("rust")).unwrap();
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_RUST_ANALYZER") {
        rust.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![rust]));
    let doc = DocId::next();
    mgr.did_open(doc, &main, &LanguageId::new("rust"), 1, MAIN_RS);
    retry("rust-analyzer ready", Duration::from_secs(60), || {
        let s = last_status(&events, "rust-analyzer");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("rust-analyzer failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;
    let servers = mgr.servers();
    assert_eq!(servers[0].root, root);

    // Hover over `add` in `add(p.x, p.y)` (line 12, col 12).
    let hover = retry("hover", Duration::from_secs(120), || {
        let mgr = mgr.clone();
        async move { mgr.hover(doc, Pos::new(12, 13), Default::default()).await.ok().flatten() }
    })
    .await;
    assert!(hover.contents.value.contains("fn add"), "{}", hover.contents.value);
    assert!(hover.contents.value.contains("Adds two numbers"));

    let defs = retry("definition", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move { mgr.goto(doc, Pos::new(12, 13), GotoKind::Definition).await.ok().filter(|d| !d.is_empty()) }
    })
    .await;
    assert_eq!(defs[0].path, main);
    assert_eq!(defs[0].range.start.line, 6);

    // Type `p.` on a new line inside main and complete fields.
    let insert_at = Pos::new(13, 0);
    mgr.did_change(
        doc,
        2,
        &[AppliedChange { range: Range::point(insert_at), text: "    p.\n".into() }],
        &|| unreachable!(),
    );
    let list = retry("completion", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move {
            let list = mgr.completion(doc, Pos::new(13, 6), Some(".".into()), Default::default()).await.ok()?;
            list.items.iter().any(|i| i.label == "x").then_some(list)
        }
    })
    .await;
    let x = list.items.iter().find(|i| i.label == "x").unwrap();
    assert_eq!(x.replace_range.map(|r| r.start), Some(Pos::new(13, 6)));
    let resolved = mgr.resolve_completion(doc, x.clone()).await.unwrap();
    assert_eq!(resolved.label, "x");

    let symbols = mgr.document_symbols(doc).await.unwrap().unwrap();
    assert!(symbols.iter().any(|s| s.name == "add"));
    let tokens = mgr.semantic_tokens(doc).await.unwrap().unwrap();
    assert!(!tokens.data.is_empty() && tokens.data.len() % 5 == 0);

    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
    assert_eq!(last_status(&events, "rust-analyzer").unwrap().0, ServerStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires typescript-language-server"]
async fn typescript_language_server_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("tsconfig.json"), "{ \"compilerOptions\": { \"strict\": true } }").unwrap();
    let file = root.join("a.ts");
    let text =
        "export function greet(name: string): string {\n  return `hi ${name}`;\n}\nconst n: number = greet(\"x\");\n";
    std::fs::write(&file, text).unwrap();
    let server = LanguageServerConfig {
        name: "typescript-language-server".into(),
        command: "typescript-language-server".into(),
        args: vec!["--stdio".into()],
        env: Default::default(),
        root_markers: vec!["tsconfig.json".into()],
        require_root_marker: false,
        initialization_options: None,
        settings: None,
        install_hint: None,
        connect: None,
        fallback: false,
        features: Vec::new(),
        code_action_kinds: Vec::new(),
        max_java: None,
        launch_error: None,
        tsserver_project: false,
        pull_diagnostics: false,
        workspace_diagnostics: false,
        pinned: None,
    };
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![lang("typescript", "ts", server)]));
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("typescript"), 1, text);

    let hover = retry("hover", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move { mgr.hover(doc, Pos::new(3, 19), Default::default()).await.ok().flatten() }
    })
    .await;
    assert!(hover.contents.value.contains("greet"), "{}", hover.contents.value);

    let defs = mgr.goto(doc, Pos::new(3, 19), GotoKind::Definition).await.unwrap();
    assert_eq!(defs[0].path, file);
    assert_eq!(defs[0].range.start.line, 0);

    // Type error: string assigned to number.
    retry("diagnostics", Duration::from_secs(30), || {
        let found = events
            .lock()
            .iter()
            .any(|e| matches!(e, IdeEvent::Diagnostics { path, items, .. } if path == &file && !items.is_empty()));
        async move { found.then_some(()) }
    })
    .await;
    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
}

const APP_VUE: &str = "<script setup lang=\"ts\">\nconst count: number = 1\n</script>\n\n<template>\n  <div>{{ count }}</div>\n</template>\n";

fn npm_server(tools: &Path, name: &str, bin: &str, init: Option<serde_json::Value>) -> LanguageServerConfig {
    LanguageServerConfig {
        name: name.into(),
        command: tools.join("node_modules/.bin").join(bin).to_string_lossy().into_owned(),
        args: vec!["--stdio".into()],
        env: Default::default(),
        root_markers: vec!["package.json".into()],
        require_root_marker: false,
        initialization_options: init.map(|v| toml::Value::try_from(v).unwrap()),
        settings: None,
        install_hint: None,
        connect: None,
        fallback: false,
        features: Vec::new(),
        code_action_kinds: Vec::new(),
        max_java: None,
        launch_error: None,
        tsserver_project: false,
        pull_diagnostics: false,
        workspace_diagnostics: false,
        pinned: None,
    }
}

/// Vue Language Server 3 in hybrid mode: its `tsserver/request`s go through the
/// client to vtsls with `@vue/typescript-plugin` (issue #25). Set
/// `OPENIDEA_TEST_NPM_TOOLS` to a dir with `npm i @vue/language-server typescript@6 @vtsls/language-server`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires @vue/language-server and @vtsls/language-server (OPENIDEA_TEST_NPM_TOOLS)"]
async fn vue_hybrid_mode_end_to_end() {
    let tools = std::path::PathBuf::from(std::env::var("OPENIDEA_TEST_NPM_TOOLS").expect("OPENIDEA_TEST_NPM_TOOLS"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("package.json"), "{\"name\":\"app\",\"private\":true}").unwrap();
    std::fs::write(root.join("tsconfig.json"), "{\"compilerOptions\":{\"strict\":true},\"include\":[\"src/**/*\"]}")
        .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/App.vue");
    std::fs::write(&file, APP_VUE).unwrap();

    let plugin = serde_json::json!({ "vtsls": { "tsserver": { "globalPlugins": [{
        "name": "@vue/typescript-plugin",
        "location": tools.join("node_modules/@vue/language-server").to_string_lossy(),
        "languages": ["vue"],
        "configNamespace": "typescript",
        "enableForWorkspaceTypeScriptVersions": true,
    }] } } });
    for with_vtsls in [false, true] {
        let mut vue = lang("vue", "vue", npm_server(&tools, "vue-language-server", "vue-language-server", None));
        if with_vtsls {
            vue.language_servers.push(npm_server(&tools, "vtsls", "vtsls", Some(plugin.clone())));
        }
        let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![vue]));
        let doc = DocId::next();
        mgr.did_open(doc, &file, &LanguageId::new("vue"), 1, APP_VUE);
        // `count` in `{{ count }}`.
        let pos = Pos::new(5, 12);
        if with_vtsls {
            let hover = retry("hover", Duration::from_secs(90), || {
                let mgr = mgr.clone();
                async move { mgr.hover(doc, pos, Default::default()).await.ok().flatten() }
            })
            .await;
            assert!(hover.contents.value.contains("count: number"), "{}", hover.contents.value);
        }
        retry("vue-language-server ready", Duration::from_secs(30), || {
            let s = last_status(&events, "vue-language-server");
            async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
        })
        .await;
        // Every request of vue-language-server waits for a tsserver answer (the
        // project info); without vtsls it is `null` and the server falls back.
        let symbols = tokio::time::timeout(Duration::from_secs(20), mgr.document_symbols(doc)).await;
        assert!(matches!(symbols, Ok(Ok(_))), "vue-language-server did not answer: {symbols:?}");
        let crashed = matches!(last_status(&events, "vue-language-server"), Some((ServerStatus::Crashed, _)));
        assert!(!crashed, "{:?}", last_status(&events, "vue-language-server"));
        tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
    }
}

const NG_APP_TS: &str = "import { Component } from '@angular/core';\n\n@Component({\n  selector: 'app-root',\n  templateUrl: './app.html',\n})\nexport class App {\n  title = 'demo';\n}\n";
const NG_APP_HTML: &str = "<h1>{{ title }}</h1>\n<p>{{ missing }}</p>\n";
const NG_INLINE_TS: &str = "import { Component } from '@angular/core';\n\n@Component({\n  selector: 'app-inline',\n  template: `<b>{{ count }}</b>`,\n})\nexport class Inline {\n  count = 1;\n}\n";

/// The lines `edit` changes in `path`, in order.
fn edited_lines(edit: &ide_core::WorkspaceEdit, path: &Path) -> Vec<u32> {
    let mut lines: Vec<u32> = edit
        .files
        .iter()
        .filter_map(|f| match f {
            ide_core::FileEdit::Edit { path: p, edits, .. } if p.to_path_buf() == path => {
                Some(edits.iter().map(|e| e.range.start.line))
            }
            _ => None,
        })
        .flatten()
        .collect();
    lines.sort();
    lines
}

/// The Angular Language Server next to vtsls, in an external and an inline
/// template. Set `OPENIDEA_TEST_NPM_TOOLS` to a dir with `npm i
/// @angular/language-server @vtsls/language-server @angular/core @angular/common
/// @angular/compiler rxjs`; the project is created inside it, so that it
/// resolves `@angular/core`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires @angular/language-server, @vtsls/language-server and @angular/core (OPENIDEA_TEST_NPM_TOOLS)"]
async fn angular_language_server_end_to_end() {
    let tools = std::path::PathBuf::from(std::env::var("OPENIDEA_TEST_NPM_TOOLS").expect("OPENIDEA_TEST_NPM_TOOLS"));
    let dir = tempfile::tempdir_in(&tools).unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("angular.json"), "{\"version\":1,\"projects\":{}}").unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true},\"angularCompilerOptions\":{\"strictTemplates\":true},\"include\":[\"src/**/*.ts\"]}",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    let app_ts = root.join("src/app/app.ts");
    let app_html = root.join("src/app/app.html");
    let inline_ts = root.join("src/app/inline.ts");
    for (path, text) in [(&app_ts, NG_APP_TS), (&app_html, NG_APP_HTML), (&inline_ts, NG_INLINE_TS)] {
        std::fs::write(path, text).unwrap();
    }

    // The built-in definitions, with vtsls and the Angular server, and the probe
    // locations the settings hook adds.
    let builtin = LanguageRegistry::load(None).unwrap();
    let probes = tools.join("node_modules/@angular/language-server").to_string_lossy().into_owned();
    let languages = ["typescript", "html"].map(|id| {
        let mut cfg = builtin.get(&LanguageId::new(id)).unwrap();
        cfg.language_servers.retain(|s| s.name == "angular-language-server" || s.name == "vtsls");
        for server in &mut cfg.language_servers {
            let bin = if server.name == "vtsls" { "vtsls" } else { "ngserver" };
            server.command = tools.join("node_modules/.bin").join(bin).to_string_lossy().into_owned();
        }
        let server = cfg.language_servers.iter_mut().find(|s| s.name == "angular-language-server").unwrap();
        server.args.extend(["--ngProbeLocations".into(), probes.clone(), "--tsProbeLocations".into(), probes.clone()]);
        cfg
    });
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(languages.to_vec()));
    let (html, ts, inline) = (DocId::next(), DocId::next(), DocId::next());
    mgr.did_open(ts, &app_ts, &LanguageId::new("typescript"), 1, NG_APP_TS);
    mgr.did_open(html, &app_html, &LanguageId::new("html"), 1, NG_APP_HTML);
    mgr.did_open(inline, &inline_ts, &LanguageId::new("typescript"), 1, NG_INLINE_TS);

    // `title` in `{{ title }}`.
    let hover = retry("hover", Duration::from_secs(90), || {
        let mgr = mgr.clone();
        async move { mgr.hover(html, Pos::new(0, 8), Default::default()).await.ok().flatten() }
    })
    .await;
    assert!(hover.contents.value.contains("title"), "{}", hover.contents.value);
    let defs = mgr.goto(html, Pos::new(0, 8), GotoKind::Definition).await.unwrap();
    assert_eq!(defs[0].path, app_ts);
    assert_eq!(defs[0].range.start.line, 7);

    // `missing` is no member of `App`.
    retry("template diagnostics", Duration::from_secs(60), || {
        let found = events
            .lock()
            .iter()
            .any(|e| matches!(e, IdeEvent::Diagnostics { path, items, .. } if path == &app_html && !items.is_empty()));
        async move { found.then_some(()) }
    })
    .await;

    // `count` in the inline template, which vtsls can't rename: renamed there
    // and in the class.
    let count = Pos::new(4, 20);
    let range = mgr.prepare_rename(inline, count).await.unwrap();
    assert_eq!(range, Some(Range::new(Pos::new(4, 19), Pos::new(4, 24))));
    let edit = mgr.rename(inline, count, "total".into(), Default::default()).await.unwrap().edit;
    assert_eq!(edited_lines(&edit, &inline_ts), [4, 7], "{edit:?}");

    // `title` in the class, which both servers rename: vtsls's edit and the
    // template's from the Angular server.
    let title = Pos::new(7, 3);
    let edit = mgr.rename(ts, title, "heading".into(), Default::default()).await.unwrap().edit;
    assert_eq!((edited_lines(&edit, &app_ts), edited_lines(&edit, &app_html)), (vec![7], vec![0]), "{edit:?}");
    // Its usage in the template, the declaration left out, from either file.
    for (doc, pos) in [(ts, title), (html, Pos::new(0, 8))] {
        let refs = mgr.references(doc, pos, false, false, Default::default()).await.unwrap();
        let found: Vec<(std::path::PathBuf, u32)> =
            refs.locations.iter().map(|l| (l.path.to_path_buf(), l.range.start.line)).collect();
        assert_eq!(found, [(app_html.clone(), 0)]);
    }
    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
}

/// Copies `fixtures/dotnet` without build output, so a test can restore and edit it.
fn copy_dotnet_fixture(to: &Path) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let name = entry.file_name();
            if name == "bin" || name == "obj" {
                continue;
            }
            let (src, dst) = (entry.path(), to.join(&name));
            if src.is_dir() { copy(&src, &dst) } else { std::fs::copy(&src, &dst).map(|_| ()).unwrap() }
        }
    }
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dotnet"), to);
}

/// Roslyn's language server on the fixture solution. Needs the .NET 10 SDK and
/// `roslyn-language-server` on `PATH`, or its path in `OPENIDEA_TEST_ROSLYN`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));

    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let program = root.join("src/Fixture.Cli/Program.cs");
    let source = std::fs::read_to_string(&program).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &program, &LanguageId::new("csharp"), 1, &source);
    retry("roslyn ready", Duration::from_secs(60), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;

    // `Greet` in `greeter.Greet(...)` (line 3) is declared on IGreeter, in another project.
    let greet = Pos::new(3, 28);
    let hover = retry("hover", Duration::from_secs(180), || {
        let mgr = mgr.clone();
        async move { mgr.hover(doc, greet, Default::default()).await.ok().flatten() }
    })
    .await;
    assert!(hover.contents.value.contains("Greet"), "{}", hover.contents.value);
    let defs = retry("definition", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move { mgr.goto(doc, greet, GotoKind::Definition).await.ok().filter(|d| !d.is_empty()) }
    })
    .await;
    assert_eq!(defs[0].path, root.join("src/Fixture.Core/Greeter.cs"));
    assert_eq!(defs[0].range.start.line, 4);

    // Member completion on the interface.
    let at = Pos::new(4, 0);
    mgr.did_change(doc, 2, &[AppliedChange { range: Range::point(at), text: "greeter.\n".into() }], &|| unreachable!());
    retry("completion", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move {
            let list = mgr.completion(doc, Pos::new(4, 8), Some(".".into()), Default::default()).await.ok()?;
            list.items.iter().any(|i| i.label == "Greet").then_some(())
        }
    })
    .await;

    // The incomplete statement is a compile error. The editor shows pushed
    // diagnostics only (the client does not advertise pull), so wait for a push.
    let diagnostics = retry("diagnostics", Duration::from_secs(60), || {
        let pushed = events.lock().iter().rev().find_map(|e| match e {
            IdeEvent::Diagnostics { path, items, .. } if path.0 == program && !items.is_empty() => Some(items.clone()),
            _ => None,
        });
        // The first result can predate the edit; wait for the edited line's error.
        async move { pushed.filter(|items| items.iter().any(|d| d.range.start.line == 4)) }
    })
    .await;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.severity == ide_core::Severity::Error && d.code.as_deref().is_some_and(|c| c.starts_with("CS"))),
        "{diagnostics:?}"
    );

    tokio::time::timeout(Duration::from_secs(20), mgr.shutdown_all()).await.unwrap();
}

/// CI-14: FsAutoComplete on the fixture's F# project: a type error is reported,
/// and members complete. Needs the .NET SDK and `fsautocomplete` on `PATH`, or its
/// path in `OPENIDEA_TEST_FSAC`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and fsautocomplete"]
async fn fsautocomplete_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let project = root.join("src/Fixture.FSharp");
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "Fixture.FSharp.fsproj"])
        .current_dir(&project)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));

    let mut fsharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("fsharp")).unwrap();
    fsharp.language_servers.retain(|s| s.name == "fsautocomplete");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_FSAC") {
        fsharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![fsharp]));
    let file = project.join("Library.fs");
    // The editor drops the byte order mark; the F# parser rejects it.
    let source = std::fs::read_to_string(&file).unwrap().trim_start_matches('\u{feff}').to_owned();
    let lines = source.lines().count() as u32;
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("fsharp"), 1, &source);
    retry("fsautocomplete ready", Duration::from_secs(60), || {
        let s = last_status(&events, "fsautocomplete");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("fsautocomplete failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;

    // A type error, then a member access to complete.
    let added = "    let broken : int = \"text\"\n    let size = \"abc\".Len";
    mgr.did_change(
        doc,
        2,
        &[AppliedChange { range: Range::point(Pos::new(lines, 0)), text: added.into() }],
        &|| unreachable!(),
    );
    let end = Pos::new(lines + 1, "    let size = \"abc\".Len".len() as u32);
    retry("completion", Duration::from_secs(120), || {
        let mgr = mgr.clone();
        async move {
            let list = mgr.completion(doc, end, None, Default::default()).await.ok()?;
            list.items.iter().any(|i| i.label == "Length").then_some(())
        }
    })
    .await;
    let diagnostics = retry("diagnostics", Duration::from_secs(120), || {
        let pushed = events.lock().iter().rev().find_map(|e| match e {
            IdeEvent::Diagnostics { path, items, .. } if path.0 == file && !items.is_empty() => Some(items.clone()),
            _ => None,
        });
        async move { pushed.filter(|items| items.iter().any(|d| d.range.start.line == lines)) }
    })
    .await;
    assert!(
        diagnostics.iter().any(|d| d.severity == ide_core::Severity::Error && d.range.start.line == lines),
        "{diagnostics:?}"
    );
    let hover = retry("hover", Duration::from_secs(60), || {
        let mgr = mgr.clone();
        async move { mgr.hover(doc, Pos::new(3, 10), Default::default()).await.ok().flatten() }
    })
    .await;
    assert!(hover.contents.value.contains("hello"), "{}", hover.contents.value);

    tokio::time::timeout(Duration::from_secs(20), mgr.shutdown_all()).await.unwrap();
}

/// SLN-3: a multi-targeting project has one Roslyn project context per target
/// framework, and choosing one switches which `#if` branch is live.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_project_contexts_switch_target_frameworks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));

    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, _events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let greeter = root.join("src/Fixture.Core/Greeter.cs");
    let source = std::fs::read_to_string(&greeter).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &greeter, &LanguageId::new("csharp"), 1, &source);
    let contexts = retry("project contexts", Duration::from_secs(180), || {
        let mgr = mgr.clone();
        async move { mgr.project_contexts(doc).await.ok().flatten().filter(|c| c.contexts.len() == 2) }
    })
    .await;
    let labels: Vec<&str> = contexts.contexts.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("net10.0")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("netstandard2.0")), "{labels:?}");

    // Line 12 (0-based) is `ArgumentException.ThrowIfNullOrWhiteSpace(name);` under `#if NET10_0_OR_GREATER`.
    let excluded_on_line = |tokens: &ide_lsp::types::SemanticTokens, line: u32| {
        let excluded = tokens.token_types.iter().position(|t| t == "excludedCode").map(|i| i as u32);
        tokens.data.chunks(5).any(|q| q[0] == line && Some(q[3]) == excluded)
    };
    for (needle, excluded) in [("netstandard2.0", true), ("net10.0", false)] {
        let context = contexts.contexts.iter().find(|c| c.label.contains(needle)).unwrap().clone();
        mgr.set_project_context(doc, Some(context.clone()));
        let tokens = retry(needle, Duration::from_secs(60), || {
            let mgr = mgr.clone();
            async move {
                let tokens = mgr.semantic_tokens(doc).await.ok().flatten()?;
                (excluded_on_line(&tokens, 12) == excluded).then_some(tokens)
            }
        })
        .await;
        assert_eq!(excluded_on_line(&tokens, 12), excluded, "{needle}");
        let now = mgr.project_contexts(doc).await.unwrap().unwrap();
        assert_eq!(now.contexts[now.active as usize].id, context.id);
    }
    tokio::time::timeout(Duration::from_secs(10), mgr.shutdown_all()).await.unwrap();
}

/// Applies the edits of `file` in `edit` to `text` (line/column edits, ASCII text).
fn apply_file_edits(text: &str, edit: &ide_core::WorkspaceEdit, file: &Path) -> String {
    let mut edits: Vec<ide_core::TextEdit> = edit
        .files
        .iter()
        .filter_map(|f| match f {
            ide_core::FileEdit::Edit { path, edits, .. } if AsRef::<Path>::as_ref(path) == file => Some(edits.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let offset = |p: Pos| {
        let line_start: usize = text.split_inclusive('\n').take(p.line as usize).map(str::len).sum();
        line_start + p.col as usize
    };
    edits.sort_by_key(|e| std::cmp::Reverse((e.range.start.line, e.range.start.col)));
    let mut out = text.to_owned();
    for e in edits {
        out.replace_range(offset(e.range.start)..offset(e.range.end), &e.new_text);
    }
    out
}

/// CI-6: each C# refactoring Roslyn provides, from `Pricing.cs` of the fixture:
/// offered where IDEA offers it, resolved to an edit, and doing what it says.
/// Introduce Constant and Introduce Parameter come from Roslyn's action groups,
/// which the client flattens. Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_refactorings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let file = root.join("src/Fixture.Core/Pricing.cs");
    let source = std::fs::read_to_string(&file).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("csharp"), 1, &source);
    retry("roslyn ready", Duration::from_secs(60), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;

    let range = |l1, c1, l2, c2| Range { start: Pos::new(l1, c1), end: Pos::new(l2, c2) };
    // (refactoring, where, the action's title, what the file then contains)
    let cases: [(&str, Range, &str, &[&str]); 8] = [
        (
            "Extract Method",
            range(7, 0, 8, 61),
            "Extract method",
            &["NewMethod(price, quantity, out subtotal, out discount)"],
        ),
        (
            "Introduce Variable",
            range(8, 40, 8, 54),
            "Introduce local for 'subtotal * 0.1m'",
            &["decimal v = subtotal * 0.1m;"],
        ),
        ("Introduce Constant", range(8, 51, 8, 55), "Introduce constant for '0.1m'", &["const decimal"]),
        (
            "Introduce Parameter",
            range(8, 51, 8, 55),
            "Introduce parameter for '0.1m' and update call sites directly",
            &["Total(decimal price, int quantity, decimal"],
        ),
        (
            "Extract Interface",
            range(3, 21, 3, 21),
            "Extract interface...",
            &["interface IPricing", "class Pricing : IPricing"],
        ),
        (
            "Extract Superclass",
            range(3, 21, 3, 21),
            "Extract base class...",
            &["class NewBaseType", "Pricing : NewBaseType"],
        ),
        ("Inline", range(7, 13, 7, 13), "Inline temporary variable", &["price * quantity > 100"]),
        ("Move", range(15, 21, 15, 21), "Move type to Receipt.cs", &[]),
    ];
    for (name, at, title, expect) in cases {
        let action = retry(name, Duration::from_secs(120), || {
            let mgr = mgr.clone();
            async move {
                let actions = mgr.code_actions(doc, at, Vec::new()).await.ok()?;
                actions.into_iter().find(|a| a.title == title)
            }
        })
        .await;
        let resolved = mgr.resolve_code_action(doc, action).await.unwrap();
        let edit = resolved.edit.unwrap_or_else(|| panic!("{name}: no edit"));
        let after = apply_file_edits(&source, &edit, &file);
        for want in expect {
            assert!(after.contains(want), "{name}: no {want:?} in\n{after}");
        }
        if name == "Move" {
            let receipt = root.join("src/Fixture.Core/Receipt.cs");
            assert!(
                edit.files.iter().any(
                    |f| matches!(f, ide_core::FileEdit::Create { path, .. } if AsRef::<Path>::as_ref(path) == receipt)
                ),
                "{edit:?}"
            );
            let moved = apply_file_edits("", &edit, &receipt);
            assert!(moved.contains("public sealed class Receipt"), "{moved}");
            assert!(!after.contains("class Receipt"), "{after}");
        }
    }
    tokio::time::timeout(Duration::from_secs(20), mgr.shutdown_all()).await.unwrap();
}

/// CI-13: Roslyn's call hierarchy (incoming and outgoing) and type hierarchy
/// (supertypes and subtypes) for C#. Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_hierarchies() {
    use ide_lsp::types::HierarchyDirection;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let file = root.join("src/Fixture.Core/Checkout.cs");
    let source = "namespace Fixture.Core;\n\npublic interface IShape { double Area(); }\n\npublic class Circle : IShape\n{\n    public double Area() => 3.14;\n}\n\npublic static class Checkout\n{\n    public static decimal Due(Pricing p, decimal price) => p.Tax(p.Total(price, 1));\n\n    public static decimal Twice(Pricing p) => Due(p, 1) + Due(p, 2);\n}\n";
    std::fs::write(&file, source).unwrap();
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("csharp"), 1, source);
    retry("roslyn ready", Duration::from_secs(60), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;

    // `Due` (line 11): called twice from `Twice`, calls `Tax` and `Total`.
    let due = Pos::new(11, 27);
    let incoming = retry("incoming calls of Due", Duration::from_secs(90), || {
        let mgr = &mgr;
        async move {
            let h = mgr.call_hierarchy(doc, due, HierarchyDirection::Incoming, Default::default()).await.ok()?;
            (!h.items.is_empty()).then_some(h)
        }
    })
    .await;
    let callers: Vec<(&str, usize)> = incoming.items.iter().map(|i| (i.name.as_str(), i.from_ranges.len())).collect();
    assert!(callers.iter().any(|(n, k)| n.contains("Twice") && *k == 2), "{callers:?}");

    let outgoing = retry("outgoing calls of Due", Duration::from_secs(30), || {
        let mgr = &mgr;
        async move {
            let h = mgr.call_hierarchy(doc, due, HierarchyDirection::Outgoing, Default::default()).await.ok()?;
            (!h.items.is_empty()).then_some(h)
        }
    })
    .await;
    let mut callees: Vec<&str> = outgoing.items.iter().map(|i| i.name.as_str()).collect();
    callees.sort();
    assert!(callees.iter().any(|n| n.contains("Tax")) && callees.iter().any(|n| n.contains("Total")), "{callees:?}");

    // `Circle` (line 4): its supertype is `IShape`; `IShape`'s subtype is `Circle`.
    let supers = mgr.type_hierarchy(doc, Pos::new(4, 14), HierarchyDirection::Incoming).await.unwrap();
    assert!(
        supers.iter().any(|i| i.name.contains("IShape")),
        "{:?}",
        supers.iter().map(|i| &i.name).collect::<Vec<_>>()
    );
    let subs = mgr.type_hierarchy(doc, Pos::new(2, 18), HierarchyDirection::Outgoing).await.unwrap();
    assert!(subs.iter().any(|i| i.name.contains("Circle")), "{:?}", subs.iter().map(|i| &i.name).collect::<Vec<_>>());
}

/// CI-6: Change Signature for C# from Roslyn's references: the declaration,
/// the interface member it implements and the calls in two files are
/// rewritten. Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_change_signature() {
    use ide_analysis::signature::{NewParameter, SignatureSites, change_signature, signature_at};
    use ide_core::{FileEdit, Range};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let file = root.join("src/Fixture.Core/Shop.cs");
    let caller = root.join("src/Fixture.Core/Caller.cs");
    let source = "namespace Fixture.Core;\n\npublic interface IPricer { decimal Price(decimal net, int count); }\n\npublic class Shop : IPricer\n{\n    public decimal Price(decimal net, int count) => net * count;\n\n    public decimal Two() => Price(1m, 2) + Price(count: 3, net: 4m);\n}\n";
    let other = "namespace Fixture.Core;\n\npublic static class Caller\n{\n    public static decimal Run(IPricer p) => p.Price(5m, 6);\n}\n";
    std::fs::write(&file, source).unwrap();
    std::fs::write(&caller, other).unwrap();
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("csharp"), 1, source);
    retry("roslyn ready", Duration::from_secs(60), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;

    let syntax = ide_language::Syntax::new();
    // `Shop.Price` (line 6).
    let sig = signature_at(&syntax, source, Pos::new(6, 20)).unwrap();
    assert_eq!(sig.name, "Price");
    let refs = retry("references of Shop.Price", Duration::from_secs(90), || {
        let mgr = &mgr;
        let at = sig.name_range.start;
        async move {
            let r = mgr.references(doc, at, true, true, Default::default()).await.ok()?;
            // Both calls in Shop.cs and the one through the interface in Caller.cs.
            (r.locations.iter().filter(|l| l.path.ends_with("Shop.cs")).count() >= 3
                && r.locations.iter().any(|l| l.path.ends_with("Caller.cs")))
            .then_some(r)
        }
    })
    .await;
    let mut files: Vec<SignatureSites> = Vec::new();
    for (path, text) in [(&file, source), (&caller, other)] {
        let mut locations: Vec<Range> =
            refs.locations.iter().filter(|l| &*l.path == path.as_path()).map(|l| l.range).collect();
        if path == &file {
            locations.push(sig.name_range);
        }
        files.push(SignatureSites { path: path.as_path().into(), text: text.into(), locations });
    }
    let change = [
        NewParameter { from: Some(1), text: "int count".into(), value: None },
        NewParameter { from: Some(0), text: "decimal net".into(), value: None },
        NewParameter { from: None, text: "bool round = false".into(), value: None },
    ];
    let rewrite = change_signature(&syntax, &sig, &change, &files).unwrap();
    assert!(rewrite.skipped.is_empty(), "{:?}", rewrite.skipped);
    let apply = |path: &std::path::Path, text: &str| {
        let Some(FileEdit::Edit { edits, .. }) =
            rewrite.edit.files.iter().find(|f| matches!(f, FileEdit::Edit { path: p, .. } if &**p == path))
        else {
            return text.to_string();
        };
        let mut out = text.to_string();
        let mut edits = edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
        let byte = |t: &str, p: Pos| {
            let start: usize = t.split('\n').take(p.line as usize).map(|l| l.len() + 1).sum();
            start + p.col as usize
        };
        for e in edits {
            let (s, t) = (byte(&out, e.range.start), byte(&out, e.range.end));
            out.replace_range(s..t, &e.new_text);
        }
        out
    };
    let shop = apply(&file, source);
    assert!(shop.contains("public decimal Price(int count, decimal net, bool round = false) =>"), "{shop}");
    assert!(shop.contains("Price(2, 1m) + Price(count: 3, net: 4m)"), "{shop}");
    let calls = apply(&caller, other);
    assert!(calls.contains("p.Price(6, 5m)"), "{calls}");
    // Roslyn cascades to the interface member, so it changes with its implementation.
    assert!(shop.contains("decimal Price(int count, decimal net, bool round = false); }"), "{shop}");
    assert_eq!((rewrite.declarations, rewrite.calls), (2, 3));
}

/// CI-15: Razor through Roslyn's cohosted Razor support: component completion in
/// a `.razor` file of a new Blazor app. Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_razor_completion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let new = std::process::Command::new("dotnet")
        .args(["new", "blazor", "-n", "BlazorApp", "-o", "."])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(new.status.success(), "{}", String::from_utf8_lossy(&new.stdout));
    let registry = LanguageRegistry::load(None).unwrap();
    let mut csharp = registry.get(&LanguageId::new("csharp")).unwrap();
    let mut razor = registry.get(&LanguageId::new("razor")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin.clone();
        razor.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp, razor]));
    let file = root.join("Components/Pages/Home.razor");
    let source = "@page \"/\"\n\n<PageTitle>Home</PageTitle>\n\n<Cou\n";
    std::fs::write(&file, source).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &file, &LanguageId::new("razor"), 1, source);
    retry("roslyn ready", Duration::from_secs(60), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        async move { matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _))).then_some(()) }
    })
    .await;
    let list = retry("Counter in the completion", Duration::from_secs(120), || {
        let mgr = &mgr;
        async move {
            let list = mgr.completion(doc, Pos::new(4, 4), None, Default::default()).await.ok()?;
            list.items.iter().any(|i| i.label == "Counter").then_some(list)
        }
    })
    .await;
    // The app's own components and the framework's.
    assert!(
        list.items.iter().any(|i| i.label == "Weather"),
        "{:?}",
        list.items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );
    assert!(list.items.iter().any(|i| i.label.ends_with("AuthorizeRouteView")));
}

/// NFR-4: Roslyn is killed while a file has unsaved edits. Editing goes on while it
/// is down, it restarts on its own, and the restarted server is given the editor's
/// text (not the file on disk). Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_restarts_after_a_crash_with_the_unsaved_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));

    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let program = root.join("src/Fixture.Cli/Program.cs");
    let source = std::fs::read_to_string(&program).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &program, &LanguageId::new("csharp"), 1, &source);
    let ready = |events: &Events| {
        let s = last_status(events, "roslyn-language-server");
        if let Some((ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        matches!(s, Some((ServerStatus::Ready | ServerStatus::Busy, _)))
    };
    retry("roslyn ready", Duration::from_secs(60), || {
        let ok = ready(&events);
        async move { ok.then_some(()) }
    })
    .await;
    let hover_at = |pos: Pos, needle: &'static str| {
        let mgr = mgr.clone();
        async move {
            let h = mgr.hover(doc, pos, Default::default()).await.ok().flatten()?;
            h.contents.value.contains(needle).then_some(h.contents.value)
        }
    };
    retry("hover before the crash", Duration::from_secs(180), || hover_at(Pos::new(3, 28), "Greet")).await;

    // Kill the server: our only child process (the launcher `exec`s it).
    let children = std::process::Command::new("pgrep").args(["-P", &std::process::id().to_string()]).output().unwrap();
    let pids: Vec<String> = String::from_utf8_lossy(&children.stdout).split_whitespace().map(str::to_owned).collect();
    assert!(!pids.is_empty(), "no server process");
    let before = events.lock().len();
    for pid in &pids {
        std::process::Command::new("kill").args(["-KILL", pid]).status().unwrap();
    }
    retry("the crash is reported", Duration::from_secs(20), || {
        let crashed = events.lock()[before..].iter().any(|e| {
            matches!(e, IdeEvent::LanguageServer { name, status: ServerStatus::Crashed, .. } if name == "roslyn-language-server")
        });
        async move { crashed.then_some(()) }
    })
    .await;

    // Typing while it is down: a line that is only in the editor.
    let line = "var shout = greeter.Greet(\"x\");\n";
    mgr.did_change(
        doc,
        2,
        &[AppliedChange { range: Range::point(Pos::new(4, 0)), text: line.into() }],
        &|| unreachable!(),
    );
    assert_eq!(std::fs::read_to_string(&program).unwrap(), source, "nothing is saved");

    retry("roslyn restarted", Duration::from_secs(90), || {
        let ok = ready(&events);
        async move { ok.then_some(()) }
    })
    .await;
    let hover =
        retry("hover on the unsaved line", Duration::from_secs(180), || hover_at(Pos::new(4, 6), "shout")).await;
    println!("after the restart: {hover}");

    tokio::time::timeout(Duration::from_secs(20), mgr.shutdown_all()).await.unwrap();
}

/// NFR-3: how long Roslyn takes to answer member completion once the solution is
/// loaded (the popup adds the client's share, `completion` in `perf/scenarios.ts`).
/// Needs what [`roslyn_end_to_end`] needs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the .NET SDK and roslyn-language-server"]
async fn roslyn_completion_latency() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dotnet_fixture(&root);
    let restore = std::process::Command::new("dotnet")
        .args(["restore", "DotnetFixture.slnx"])
        .current_dir(&root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));

    let mut csharp = LanguageRegistry::load(None).unwrap().get(&LanguageId::new("csharp")).unwrap();
    csharp.language_servers.retain(|s| s.name == "roslyn-language-server");
    if let Ok(bin) = std::env::var("OPENIDEA_TEST_ROSLYN") {
        csharp.language_servers[0].command = bin;
    }
    let (mgr, events) = manager(&root, LanguageRegistry::from_configs(vec![csharp]));
    let program = root.join("src/Fixture.Cli/Program.cs");
    let source = std::fs::read_to_string(&program).unwrap();
    let doc = DocId::next();
    mgr.did_open(doc, &program, &LanguageId::new("csharp"), 1, &source);
    mgr.did_change(
        doc,
        2,
        &[AppliedChange { range: Range::point(Pos::new(4, 0)), text: "greeter.\n".into() }],
        &|| unreachable!(),
    );
    let at = Pos::new(4, 8);
    // Warm: the first answers come while the projects load.
    retry("completion", Duration::from_secs(240), || {
        let s = last_status(&events, "roslyn-language-server");
        if let Some((ServerStatus::Crashed | ServerStatus::Missing | ServerStatus::Misconfigured, msg)) = &s {
            panic!("roslyn failed: {msg:?}");
        }
        let mgr = mgr.clone();
        async move {
            let list = mgr.completion(doc, at, Some(".".into()), Default::default()).await.ok()?;
            list.items.iter().any(|i| i.label == "Greet").then_some(())
        }
    })
    .await;
    for _ in 0..5 {
        mgr.completion(doc, at, Some(".".into()), Default::default()).await.unwrap();
    }
    let mut ms: Vec<f64> = Vec::new();
    for _ in 0..40 {
        let t = Instant::now();
        let list = mgr.completion(doc, at, Some(".".into()), Default::default()).await.unwrap();
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(list.items.iter().any(|i| i.label == "Greet"));
    }
    ms.sort_by(f64::total_cmp);
    let p = |q: f64| ms[((q * ms.len() as f64).ceil() as usize).clamp(1, ms.len()) - 1];
    println!("roslyn member completion: p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms", p(0.5), p(0.95), p(1.0));
    // With the client's ~60 ms, the popup stays under NFR-3's 150 ms.
    assert!(p(0.95) < 90.0, "p95 {:.1} ms", p(0.95));

    tokio::time::timeout(Duration::from_secs(20), mgr.shutdown_all()).await.unwrap();
}
