use ide_core::{IdeError, IdeResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: ide_core::IpcPath,
    pub name: String,
    pub opened_at_ms: f64,
    /// Current git branch when last opened, shown on the welcome screen.
    pub branch: Option<String>,
}

pub const MAX_RECENT_PROJECTS: usize = 30;

/// Persisted at `<app-config>/recent.json`, newest first.
pub struct RecentProjects {
    file: PathBuf,
    items: Mutex<Vec<RecentProject>>,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

impl RecentProjects {
    pub fn load(config_dir: &Path) -> Self {
        let file = config_dir.join("recent.json");
        let mut items: Vec<RecentProject> = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("{} is invalid, ignoring: {e}", file.display());
                Vec::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::warn!("cannot read {}: {e}", file.display());
                Vec::new()
            }
        };
        items.sort_by(|a, b| b.opened_at_ms.total_cmp(&a.opened_at_ms));
        let mut seen = std::collections::HashSet::new();
        items.retain(|p| seen.insert(p.path.clone()));
        items.truncate(MAX_RECENT_PROJECTS);
        Self { file, items: Mutex::new(items) }
    }

    pub fn list(&self) -> Vec<RecentProject> {
        self.items.lock().clone()
    }

    /// Moves (or adds) `path` to the top of the list.
    pub fn touch(&self, path: &Path, branch: Option<String>) -> IdeResult<()> {
        let mut items = self.items.lock();
        items.retain(|p| p.path != path);
        let name =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        items.insert(0, RecentProject { path: path.into(), name, opened_at_ms: now_ms(), branch });
        items.truncate(MAX_RECENT_PROJECTS);
        self.persist(&items)
    }

    /// Shows `path` as `name` (File › Rename Project…); `None` restores its folder name.
    pub fn rename(&self, path: &Path, name: Option<&str>) -> IdeResult<()> {
        let mut items = self.items.lock();
        let Some(item) = items.iter_mut().find(|p| p.path == path) else { return Ok(()) };
        let name = name.map(str::to_owned).unwrap_or_else(|| {
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
        });
        if item.name == name {
            return Ok(());
        }
        item.name = name;
        self.persist(&items)
    }

    pub fn remove(&self, path: &Path) -> IdeResult<()> {
        let mut items = self.items.lock();
        let before = items.len();
        items.retain(|p| p.path != path);
        if items.len() == before {
            return Ok(());
        }
        self.persist(&items)
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.list().into_iter().map(|p| p.path.into_path_buf()).collect()
    }

    fn persist(&self, items: &[RecentProject]) -> IdeResult<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(items).map_err(|e| IdeError::internal(e.to_string()))?;
        ide_vfs::write_atomic(&self.file, &json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_capped_and_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config");
        let recent = RecentProjects::load(&cfg);
        assert!(recent.list().is_empty());
        for i in 0..35 {
            recent.touch(&PathBuf::from(format!("/p/proj{i}")), None).unwrap();
        }
        recent.touch(Path::new("/p/proj3"), Some("main".into())).unwrap();
        let list = recent.list();
        assert_eq!(list.len(), MAX_RECENT_PROJECTS);
        assert_eq!(list[0].path, PathBuf::from("/p/proj3"));
        assert_eq!(list[0].name, "proj3");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[1].path, PathBuf::from("/p/proj34"));

        recent.remove(Path::new("/p/proj34")).unwrap();
        let reloaded = RecentProjects::load(&cfg);
        assert_eq!(reloaded.paths(), recent.paths());
        assert_eq!(reloaded.list().len(), MAX_RECENT_PROJECTS - 1);
        assert!(!reloaded.paths().contains(&PathBuf::from("/p/proj34")));

        std::fs::write(cfg.join("recent.json"), "not json").unwrap();
        assert!(RecentProjects::load(&cfg).list().is_empty());
    }

    #[test]
    fn rename_shows_a_custom_name_until_reset() {
        let tmp = tempfile::tempdir().unwrap();
        let recent = RecentProjects::load(tmp.path());
        recent.touch(Path::new("/p/app"), None).unwrap();
        recent.rename(Path::new("/p/app"), Some("Shop Backend")).unwrap();
        recent.rename(Path::new("/p/unknown"), Some("x")).unwrap();
        assert_eq!(RecentProjects::load(tmp.path()).list()[0].name, "Shop Backend");
        recent.rename(Path::new("/p/app"), None).unwrap();
        assert_eq!(recent.list()[0].name, "app");
        assert_eq!(recent.list().len(), 1);
    }
}
