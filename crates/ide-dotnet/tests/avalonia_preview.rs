//! The Avalonia previewer against the real designer host. Ignored by default: it builds
//! `fixtures/avalonia/AvaloniaFixture` (Avalonia from NuGet).
//! `cargo test -p ide-dotnet --test avalonia_preview -- --ignored --nocapture`

use ide_core::CancellationToken;
use ide_dotnet::avalonia::{self, PreviewEvent, PreviewPhase};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Events = Arc<Mutex<Vec<PreviewEvent>>>;

async fn wait(events: &Events, secs: u64, pred: impl Fn(&PreviewEvent) -> bool) -> Option<PreviewEvent> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(e) = events.lock().unwrap().iter().rev().find(|e| pred(e)).cloned() {
            return Some(e);
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the .NET SDK and Avalonia from NuGet"]
async fn renders_xaml_and_reports_xaml_errors() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/avalonia/AvaloniaFixture");
    let xaml = std::fs::canonicalize(fixture.join("MainWindow.axaml")).unwrap();
    let text = std::fs::read_to_string(&xaml).unwrap();
    let events: Events = Arc::default();
    let cancel = CancellationToken::new();
    let task = {
        let (events, cancel, xaml, text) = (events.clone(), cancel.clone(), xaml.clone(), text.clone());
        tokio::spawn(async move {
            let emit = move |e: PreviewEvent| events.lock().unwrap().push(e);
            avalonia::run(1, std::path::Path::new("dotnet"), &Default::default(), &xaml, text, 2.0, &cancel, &emit)
                .await
        })
    };
    let frame = wait(&events, 180, |e| matches!(e, PreviewEvent::Frame { .. })).await;
    let Some(PreviewEvent::Frame { width, height, png, .. }) = frame else {
        panic!("no frame: {:?}", events.lock().unwrap());
    };
    println!("first frame {width}×{height}, {} base64 bytes", png.len());
    // d:DesignWidth 400 × d:DesignHeight 240 at scale 2.
    assert_eq!((width, height), (800, 480));
    assert!(events.lock().unwrap().contains(&PreviewEvent::XamlLoaded));
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&png).unwrap();
    std::fs::write(std::env::temp_dir().join("openidea-avalonia-preview.png"), &bytes).unwrap();

    events.lock().unwrap().clear();
    assert!(avalonia::update(1, text.replace("<Button", "<Buton")));
    let error = wait(&events, 30, |e| matches!(e, PreviewEvent::XamlError { .. })).await.expect("a XAML error");
    println!("{error:?}");
    let PreviewEvent::XamlError { line, .. } = error else { unreachable!() };
    let expected = text.lines().position(|l| l.contains("<Button")).map(|l| l as u32);
    assert_eq!(line, expected);

    events.lock().unwrap().clear();
    assert!(avalonia::update(1, text.replace("Click me", "Clicked")));
    wait(&events, 30, |e| matches!(e, PreviewEvent::Frame { .. })).await.expect("a frame after the fix");

    cancel.cancel();
    task.await.unwrap().unwrap();
    assert!(events.lock().unwrap().contains(&PreviewEvent::Phase { phase: PreviewPhase::Stopped, message: None }));
    assert!(!avalonia::update(1, text));
}
