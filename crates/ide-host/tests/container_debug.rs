//! Debugging .NET in a Docker container (DBG-8) end to end: an app built on the host runs
//! in `mcr.microsoft.com/dotnet/aspnet:10.0`; Attach to Process lists it, the IDE copies a
//! Linux netcoredbg into the container, attaches through `docker exec -i` and a breakpoint
//! hits. Ignored: needs Docker with that image, the .NET SDK and network access (the first
//! run downloads netcoredbg).
//! `cargo test -p ide-host --test container_debug -- --ignored --nocapture`

use ide_core::event::DebugEvent;
use ide_core::{IdeEvent, SharedSink};
use ide_dap::{Breakpoint, DebugManager, LaunchConfig, SessionOwner};
use ide_host::ops::debug::{container_processes, prepare_container};
use ide_language::LanguageRegistry;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROGRAM: &str = r#"static class Program
{
    static int Tick(int t)
    {
        Thread.Sleep(50);
        return t + 1; // tick
    }

    static void Main()
    {
        var total = 0;
        while (total < 1_000_000) total = Tick(total);
    }
}
"#;

const IMAGE: &str = "mcr.microsoft.com/dotnet/aspnet:10.0";

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

fn ok(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker, the aspnet:10.0 image, the .NET SDK and network access"]
async fn attaches_to_a_dotnet_app_in_a_container() {
    if ide_docker::docker_executable().is_none() {
        eprintln!("docker not installed");
        return;
    }
    let tag = std::process::id();
    let dir = std::env::temp_dir().join(format!("openidea-container-debug-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();
    std::fs::write(
        dir.join("Demo.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings></PropertyGroup></Project>"#,
    )
    .unwrap();
    std::fs::write(dir.join("Program.cs"), PROGRAM).unwrap();
    ok(Command::new("dotnet").args(["build", "-nologo", "-v:q"]).current_dir(&dir));

    let name = format!("openidea-debug-{tag}");
    let mount = format!("{}:/app", dir.display());
    ok(Command::new("docker").args([
        "run",
        "-d",
        "--name",
        &name,
        "-v",
        &mount,
        "-w",
        "/app",
        IMAGE,
        "dotnet",
        "bin/Debug/net10.0/Demo.dll",
    ]));
    let _container = Container(name.clone());

    // The runtime opens its diagnostics socket shortly after start.
    let deadline = Instant::now() + Duration::from_secs(20);
    let process = loop {
        let found = tokio::task::spawn_blocking(container_processes)
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.container.as_deref() == Some(name.as_str()));
        if let Some(p) = found {
            break p;
        }
        assert!(Instant::now() < deadline, "the container's .NET process was not listed");
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(process.name, "Demo");
    println!("listed: pid {} {}", process.pid, process.command_line);

    let data = dir.join("data");
    let pipe = {
        let (data, name) = (data.clone(), name.clone());
        tokio::task::spawn_blocking(move || prepare_container(&data, &name)).await.unwrap().expect("prepare")
    };
    println!("pipeTransport: {pipe}");
    let pipe: serde_json::Value = serde_json::from_str(&pipe).unwrap();

    let events: Arc<Mutex<Vec<DebugEvent>>> = Arc::default();
    let sink_events = events.clone();
    let sink: SharedSink = Arc::new(move |e: IdeEvent| {
        if let IdeEvent::Debug { event, .. } = e {
            sink_events.lock().push(event);
        }
    });
    let manager = DebugManager::new(Arc::new(LanguageRegistry::from_configs(Vec::new())));
    let source = dir.join("Program.cs");
    let tick = PROGRAM.lines().position(|l| l.contains("// tick")).unwrap() as u32;
    manager
        .set_breakpoints(
            source,
            vec![Breakpoint { line: tick, condition: None, hit_condition: None, log_message: None, enabled: true }],
        )
        .await
        .unwrap();
    let config = LaunchConfig {
        name: "container".into(),
        adapter: "netcoredbg".into(),
        request: "attach".into(),
        program: None,
        args: vec![],
        cwd: Some(dir.clone().into()),
        env: BTreeMap::new(),
        extra_json: Some(serde_json::json!({ "processId": process.pid, "pipeTransport": pipe }).to_string()),
    };
    let s = manager.start(config, SessionOwner::new("test", sink)).await.expect("attach in the container");
    let deadline = Instant::now() + Duration::from_secs(60);
    let thread = loop {
        if let Some(t) = events.lock().iter().find_map(|e| match e {
            DebugEvent::Stopped { thread_id, .. } => Some(*thread_id),
            _ => None,
        }) {
            break t;
        }
        assert!(Instant::now() < deadline, "no stop: {:?}", events.lock());
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let top = manager.stack_trace(s, thread).await.unwrap().into_iter().next().unwrap();
    assert_eq!(top.line, tick);
    assert!(top.path.as_deref().is_some_and(|p| Path::new(p).ends_with("Program.cs")), "{top:?}");
    manager.stop(s).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
