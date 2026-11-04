//! The classic text solution format:
//!
//! ```text
//! Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "src", "src", "{F1}"
//!     ProjectSection(SolutionItems) = preProject
//!         README.md = README.md
//!     EndProjectSection
//! EndProject
//! Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "App", "src\App\App.csproj", "{P1}"
//! EndProject
//! Global
//!     GlobalSection(NestedProjects) = preSolution
//!         {P1} = {F1}
//!     EndGlobalSection
//! EndGlobal
//! ```
//!
//! Entries of the solution-folder type are folders; `NestedProjects` maps a child's
//! GUID to its parent folder's.

use crate::{SolutionItem, project, resolve};
use ide_core::{IdeError, IdeResult};
use std::collections::HashMap;
use std::path::Path;

const FOLDER_TYPE: &str = "2150E333-8FDC-42A3-9474-1A3956D46DE8";

struct Entry {
    guid: String,
    item: SolutionItem,
}

pub(crate) fn parse(text: &str, dir: &Path) -> IdeResult<Vec<SolutionItem>> {
    let text = text.trim_start_matches('\u{feff}');
    if !text.lines().any(|l| l.trim_start().starts_with("Microsoft Visual Studio Solution File")) {
        return Err(IdeError::invalid("not a Visual Studio solution file"));
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut in_items = false;
    let mut in_nested = false;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Project(") {
            if let Some(entry) = project_line(rest, dir) {
                entries.push(entry);
            }
        } else if line.starts_with("ProjectSection(SolutionItems)") {
            in_items = true;
        } else if line == "EndProjectSection" {
            in_items = false;
        } else if line.starts_with("GlobalSection(NestedProjects)") {
            in_nested = true;
        } else if line == "EndGlobalSection" {
            in_nested = false;
        } else if let Some((left, right)) = line.split_once('=') {
            let (left, right) = (left.trim(), right.trim());
            if in_items && let Some(Entry { item: SolutionItem::Folder { items, .. }, .. }) = entries.last_mut() {
                items.push(SolutionItem::File { path: resolve(dir, right).into() });
            } else if in_nested {
                parents.insert(guid(left), guid(right));
            }
        }
    }
    Ok(nest(entries, &parents))
}

/// `("{type}") = "Name", "path", "{guid}"` after `Project(`.
fn project_line(rest: &str, dir: &Path) -> Option<Entry> {
    let fields: Vec<&str> = rest.split('"').collect();
    // ["", "{type}", ") = ", "Name", ", ", "path", ", ", "{guid}", ""]
    let (kind, name, path, id) = (fields.get(1)?, fields.get(3)?, fields.get(5)?, fields.get(7)?);
    let item = if guid(kind) == FOLDER_TYPE {
        SolutionItem::Folder { name: (*name).to_owned(), items: Vec::new() }
    } else {
        project(resolve(dir, path))
    };
    Some(Entry { guid: guid(id), item })
}

fn guid(s: &str) -> String {
    s.trim().trim_start_matches('{').trim_end_matches('}').to_ascii_uppercase()
}

/// Moves each entry into its parent folder; entries without one are top level.
fn nest(entries: Vec<Entry>, parents: &HashMap<String, String>) -> Vec<SolutionItem> {
    let mut children: HashMap<String, Vec<Entry>> = HashMap::new();
    let mut top: Vec<Entry> = Vec::new();
    let folders: Vec<String> =
        entries.iter().filter(|e| matches!(e.item, SolutionItem::Folder { .. })).map(|e| e.guid.clone()).collect();
    for e in entries {
        match parents.get(&e.guid).filter(|p| folders.contains(p) && **p != e.guid) {
            Some(p) => children.entry(p.clone()).or_default().push(e),
            None => top.push(e),
        }
    }
    fn build(e: Entry, children: &mut HashMap<String, Vec<Entry>>, depth: usize) -> SolutionItem {
        match e.item {
            // A nesting cycle in a hand-edited file stops at a sane depth.
            SolutionItem::Folder { name, mut items } if depth < 64 => {
                for child in children.remove(&e.guid).unwrap_or_default() {
                    items.push(build(child, children, depth + 1));
                }
                SolutionItem::Folder { name, items }
            }
            item => item,
        }
    }
    top.into_iter().map(|e| build(e, &mut children, 0)).collect()
}
