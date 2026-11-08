//! User secrets (PLT-4): the `secrets.json` that `Microsoft.Extensions.Configuration.UserSecrets`
//! reads for a project's `UserSecretsId`, outside the source tree. Opening it in the
//! editor is how Rider and Visual Studio manage secrets; a project without an id gets
//! one from `dotnet user-secrets init` first.

use crate::generators::parse_properties;
use crate::nuget::run;
use ide_core::{IdeError, IdeResult};
use std::path::{Path, PathBuf};

/// The `<UserSecretsId>` written in the project file itself.
pub fn id_in_project(text: &str) -> Option<String> {
    let rest = text.split("<UserSecretsId>").nth(1)?;
    let id = rest.split('<').next()?.trim();
    valid_id(id).then(|| id.to_owned())
}

/// An id is a folder name: nothing that would leave the secrets folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains(['/', '\\', ':'])
        && !id.contains("$(")
        && !id.chars().any(char::is_control)
}

/// Where the configuration provider looks: `%APPDATA%\Microsoft\UserSecrets\<id>\secrets.json`
/// on Windows, `~/.microsoft/usersecrets/<id>/secrets.json` elsewhere.
pub fn secrets_file(id: &str, windows: bool, appdata: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    let dir = if windows {
        appdata?.join("Microsoft").join("UserSecrets")
    } else {
        home?.join(".microsoft").join("usersecrets")
    };
    Some(dir.join(id).join("secrets.json"))
}

fn secrets_file_here(id: &str) -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    secrets_file(id, cfg!(windows), appdata.as_deref(), dirs::home_dir().as_deref())
}

/// The project's id: from its file, else as MSBuild evaluates it (`Directory.Build.props`, …).
fn evaluated_id(dotnet: Option<&Path>, project: &Path) -> IdeResult<Option<String>> {
    if let Some(id) = std::fs::read_to_string(project).ok().as_deref().and_then(id_in_project) {
        return Ok(Some(id));
    }
    let cwd = project.parent().unwrap_or(project);
    let args = ["msbuild".to_owned(), project.to_string_lossy().into_owned(), "-getProperty:UserSecretsId".into()];
    let out = run(dotnet, &Default::default(), cwd, &args)?;
    // One property name prints its bare value; several print JSON.
    let id = parse_properties(&out)
        .into_iter()
        .find(|(k, _)| k == "UserSecretsId")
        .map(|(_, v)| v)
        .unwrap_or_else(|| out.trim().to_owned());
    Ok(valid_id(&id).then_some(id))
}

/// The `secrets.json` of `project`, created (with an id in the project when it has
/// none) so it can be edited.
pub fn ensure_secrets_file(dotnet: Option<&Path>, project: &Path) -> IdeResult<PathBuf> {
    let id = match evaluated_id(dotnet, project)? {
        Some(id) => id,
        None => {
            let cwd = project.parent().unwrap_or(project);
            let args = ["user-secrets", "init", "--project"].map(String::from);
            run(dotnet, &Default::default(), cwd, &[&args[..], &[project.to_string_lossy().into_owned()]].concat())?;
            evaluated_id(dotnet, project)?
                .ok_or_else(|| IdeError::unavailable("dotnet user-secrets init did not add a UserSecretsId"))?
        }
    };
    let file = secrets_file_here(&id).ok_or_else(|| IdeError::unavailable("no home folder for user secrets"))?;
    if !file.exists() {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| IdeError::unavailable(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&file, "{\n}\n").map_err(|e| IdeError::unavailable(format!("{}: {e}", file.display())))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_the_project_file_and_stay_in_the_secrets_folder() {
        let text =
            "<Project><PropertyGroup>\n  <UserSecretsId> aspnet-Web-3f2a </UserSecretsId>\n</PropertyGroup></Project>";
        assert_eq!(id_in_project(text).as_deref(), Some("aspnet-Web-3f2a"));
        assert_eq!(id_in_project("<Project/>"), None);
        assert_eq!(id_in_project("<UserSecretsId>$(Name)-x</UserSecretsId>"), None, "MSBuild evaluates that");
        for bad in ["", "..", "a/b", "a\\b", "c:x"] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn secrets_live_where_the_configuration_provider_reads_them() {
        let unix = secrets_file("abc", false, None, Some(Path::new("/home/u"))).unwrap();
        assert_eq!(unix, Path::new("/home/u/.microsoft/usersecrets/abc/secrets.json"));
        let win = secrets_file("abc", true, Some(Path::new("/AppData/Roaming")), None).unwrap();
        assert_eq!(win, Path::new("/AppData/Roaming/Microsoft/UserSecrets/abc/secrets.json"));
        assert_eq!(secrets_file("../x", false, None, Some(Path::new("/home/u"))), None);
    }

    /// Real SDK: a project without an id gets one, and an empty `secrets.json`. Needs `dotnet`.
    #[test]
    #[ignore]
    fn a_project_without_secrets_gets_them() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Web.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n").unwrap();
        let file = ensure_secrets_file(None, &project).unwrap();
        let id = id_in_project(&std::fs::read_to_string(&project).unwrap()).expect("init wrote an id");
        assert_eq!(file, secrets_file_here(&id).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\n}\n");
        // The second time it is found, not created.
        assert_eq!(ensure_secrets_file(None, &project).unwrap(), file);
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}
