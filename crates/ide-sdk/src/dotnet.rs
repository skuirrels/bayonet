//! .NET installs and their SDKs.
//!
//! A .NET install (`DOTNET_ROOT`) holds the `dotnet` host and any number of SDKs
//! under `sdk/<version>/`. Settings choose an install; the host then picks the SDK
//! per project from `global.json`. So an [`Sdk`] here is one install: `path` and
//! `home` are the install directory (what `languages.dotnet.root` stores), and
//! `version` / `extra` the SDK the project gets from it (its own directory), or the
//! newest without a project.

use crate::exec::{query_in, which};
use crate::{Sdk, exe, home_dir};
use ide_core::version_managers;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Every install found. With a project, each names the SDK it would build that
/// project with, and installs that cannot satisfy its `global.json` come last.
pub(crate) fn detect(root: Option<&Path>) -> Vec<Sdk> {
    let mut out: Vec<(Sdk, bool)> =
        installs().iter().filter_map(|(home, source)| install(home, source, root)).collect();
    out.sort_by_key(|(_, satisfied)| !satisfied);
    out.into_iter().map(|(sdk, _)| sdk).collect()
}

/// Install directories, with where each was found; the first spelling of one wins.
fn installs() -> Vec<(PathBuf, &'static str)> {
    let mut found: Vec<(PathBuf, &'static str)> = Vec::new();
    if let Some(root) = std::env::var_os("DOTNET_ROOT") {
        found.push((PathBuf::from(root), "DOTNET_ROOT"));
    }
    // `/usr/local/bin/dotnet` is usually a link into the install.
    if let Some(host) = which("dotnet").filter(|p| !version_managers::is_shim(p))
        && let Some(dir) = host.canonicalize().ok().and_then(|p| p.parent().map(Path::to_path_buf))
    {
        found.push((dir, "path"));
    }
    for dir in system_dirs() {
        found.push((dir, "system"));
    }
    if let Some(home) = home_dir() {
        found.push((home.join(".dotnet"), "user"));
    }
    let mut seen = HashSet::new();
    found.retain(|(dir, _)| {
        dir.join(exe("dotnet")).is_file() && seen.insert(dir.canonicalize().unwrap_or_else(|_| dir.clone()))
    });
    found
}

fn system_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|p| PathBuf::from(p).join("dotnet"))
            .collect()
    } else if cfg!(target_os = "macos") {
        ["/usr/local/share/dotnet", "/opt/homebrew/opt/dotnet/libexec", "/usr/local/opt/dotnet/libexec"]
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        ["/usr/share/dotnet", "/usr/lib/dotnet", "/usr/lib64/dotnet", "/opt/dotnet", "/snap/dotnet-sdk/current"]
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }
}

/// SDK versions of one install, newest first: `sdk/<version>/` directories that
/// hold `dotnet.dll`.
pub(crate) fn sdk_versions(home: &Path) -> Vec<String> {
    let mut versions: Vec<String> = std::fs::read_dir(home.join("sdk"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|dir| dir.join("dotnet.dll").is_file())
        .filter_map(|dir| dir.file_name()?.to_str().map(str::to_owned))
        .filter(|v| parse(v).is_some())
        .collect();
    versions.sort_by(|a, b| compare(b, a));
    versions
}

/// One install as an [`Sdk`], and whether it satisfies `root`'s `global.json`.
fn install(home: &Path, source: &str, root: Option<&Path>) -> Option<(Sdk, bool)> {
    let versions = sdk_versions(home);
    let newest = versions.first()?.clone();
    let host = home.join(exe("dotnet"));
    // `dotnet --version` applies the `global.json` pin and its roll-forward policy;
    // it fails when this install has no SDK that satisfies them.
    let resolved = root.map(|root| {
        query_in(&host, &["--version"], Some(root), &[("DOTNET_CLI_TELEMETRY_OPTOUT", "1")], VERSION_TIMEOUT)
            .map(|v| v.trim().to_owned())
            .filter(|v| versions.contains(v))
    });
    let satisfied = !matches!(resolved, Some(None));
    let version = resolved.flatten().unwrap_or(newest);
    let others = versions.len() - 1;
    let name = match others {
        0 => format!(".NET SDK {version}"),
        1 => format!(".NET SDK {version} (+1 other SDK)"),
        n => format!(".NET SDK {version} (+{n} other SDKs)"),
    };
    Some((
        Sdk {
            path: home.to_path_buf().into(),
            name,
            extra: Some(home.join("sdk").join(&version).into()),
            version: Some(version),
            source: source.to_owned(),
            home: Some(home.to_path_buf().into()),
        },
        satisfied,
    ))
}

/// `10.0.403` or `10.0.100-rc.2.25502.107`: the numeric part and the prerelease label.
fn parse(version: &str) -> Option<(Vec<u64>, Option<&str>)> {
    let (numbers, pre) = match version.split_once('-') {
        Some((n, p)) => (n, Some(p)),
        None => (version, None),
    };
    let parts: Option<Vec<u64>> = numbers.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() >= 2).map(|p| (p, pre))
}

/// SemVer order: a prerelease sorts before its release.
pub(crate) fn compare(a: &str, b: &str) -> Ordering {
    match (parse(a), parse(b)) {
        (Some((na, pa)), Some((nb, pb))) => na.cmp(&nb).then_with(|| match (pa, pb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => x.cmp(y),
        }),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_install(home: &Path, versions: &[&str]) {
        std::fs::create_dir_all(home).unwrap();
        std::fs::write(home.join(exe("dotnet")), "").unwrap();
        for v in versions {
            let dir = home.join("sdk").join(v);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("dotnet.dll"), "").unwrap();
        }
    }

    #[test]
    fn an_install_is_named_after_its_newest_sdk_without_a_project() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("dotnet");
        fake_install(&home, &["8.0.416", "10.0.100-rc.2.25502.107", "10.0.403", "10.0.100"]);
        // Not SDKs: no dotnet.dll, or not a version.
        std::fs::create_dir_all(home.join("sdk/9.0.100")).unwrap();
        std::fs::create_dir_all(home.join("sdk/NuGetFallbackFolder")).unwrap();
        assert_eq!(sdk_versions(&home), ["10.0.403", "10.0.100", "10.0.100-rc.2.25502.107", "8.0.416"]);
        let (sdk, satisfied) = install(&home, "system", None).unwrap();
        assert!(satisfied);
        assert_eq!(sdk.name, ".NET SDK 10.0.403 (+3 other SDKs)");
        assert_eq!(sdk.version.as_deref(), Some("10.0.403"));
        assert_eq!(sdk.path.as_ref(), home.as_path());
        assert_eq!(sdk.home.as_deref(), Some(home.as_path()));
        assert_eq!(sdk.extra.as_deref(), Some(home.join("sdk/10.0.403").as_path()));
    }

    #[test]
    fn an_install_without_sdks_is_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        fake_install(tmp.path(), &[]);
        assert!(install(tmp.path(), "system", None).is_none());
    }

    #[test]
    fn prereleases_sort_before_their_release() {
        assert_eq!(compare("10.0.100-preview.7", "10.0.100"), Ordering::Less);
        assert_eq!(compare("10.0.100-rc.1", "10.0.100-preview.7"), Ordering::Greater);
        assert_eq!(compare("10.0.403", "9.0.306"), Ordering::Greater);
    }

    /// Against the machine's own .NET, when there is one.
    #[test]
    fn a_project_gets_the_sdk_its_global_json_pins() {
        let Some(sdk) = detect(None).into_iter().next() else { return };
        let home = sdk.home.unwrap();
        let Some(oldest) = sdk_versions(&home).pop() else { return };
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("global.json"),
            format!(r#"{{ "sdk": {{ "version": "{oldest}", "rollForward": "disable" }} }}"#),
        )
        .unwrap();
        let (pinned, satisfied) = install(&home, "system", Some(tmp.path())).unwrap();
        assert!(satisfied);
        assert_eq!(pinned.version.as_deref(), Some(oldest.as_str()));
        assert_eq!(pinned.extra.as_deref(), Some(home.join("sdk").join(&oldest).as_path()));

        std::fs::write(
            tmp.path().join("global.json"),
            r#"{ "sdk": { "version": "1.0.0", "rollForward": "disable" } }"#,
        )
        .unwrap();
        let (_, satisfied) = install(&home, "system", Some(tmp.path())).unwrap();
        assert!(!satisfied);
    }
}
