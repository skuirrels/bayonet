//! The XML solution format:
//!
//! ```xml
//! <Solution>
//!   <Folder Name="/src/">
//!     <File Path="README.md" />
//!     <Project Path="src/App/App.csproj" />
//!   </Folder>
//!   <Folder Name="/src/libs/" />
//!   <Project Path="Tool/Tool.csproj" />
//! </Solution>
//! ```
//!
//! Folders are flat in the file; `Name` is the full path, so `/src/libs/` sits
//! inside `/src/`.

use crate::{SolutionItem, project, resolve};
use ide_core::{IdeError, IdeResult};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::path::Path;

pub(crate) fn parse(text: &str, dir: &Path) -> IdeResult<Vec<SolutionItem>> {
    let invalid = |e: &dyn std::fmt::Display| IdeError::invalid(format!("not a valid .slnx: {e}"));
    let mut reader = Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut root: Vec<SolutionItem> = Vec::new();
    // The folder whose children are being read: its path segments.
    let mut folder: Option<Vec<String>> = None;
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| invalid(&e))?;
        let (tag, empty) = match &event {
            Event::Start(t) => (t, false),
            Event::Empty(t) => (t, true),
            Event::End(t) => {
                depth = depth.saturating_sub(1);
                if t.name().as_ref() == "Folder" {
                    folder = None;
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if !empty {
            depth += 1;
        }
        match tag.name().as_ref() {
            "Folder" => {
                let segments: Vec<String> = attr(tag, "Name")?
                    .unwrap_or_default()
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                folder_at(&mut root, &segments);
                if !empty {
                    folder = Some(segments);
                }
            }
            "Project" => {
                let Some(path) = attr(tag, "Path")? else { continue };
                let item = project(resolve(dir, &path));
                match &folder {
                    Some(segments) => folder_at(&mut root, segments).push(item),
                    None => root.push(item),
                }
            }
            "File" => {
                let Some(segments) = &folder else { continue };
                let Some(path) = attr(tag, "Path")? else { continue };
                folder_at(&mut root, segments).push(SolutionItem::File { path: resolve(dir, &path).into() });
            }
            _ => {}
        }
    }
    Ok(root)
}

fn attr(tag: &BytesStart, name: &str) -> IdeResult<Option<String>> {
    let found = tag.try_get_attribute(name).map_err(|e| IdeError::invalid(format!("not a valid .slnx: {e}")))?;
    found
        .map(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .map_err(|e| IdeError::invalid(format!("not a valid .slnx: {e}")))
        })
        .transpose()
}

/// The children of the folder at `segments`, creating the folders on the way.
pub(crate) fn folder_at<'a>(items: &'a mut Vec<SolutionItem>, segments: &[String]) -> &'a mut Vec<SolutionItem> {
    let Some((first, rest)) = segments.split_first() else { return items };
    let i = match items.iter().position(|i| matches!(i, SolutionItem::Folder { name, .. } if name == first)) {
        Some(i) => i,
        None => {
            items.push(SolutionItem::Folder { name: first.clone(), items: Vec::new() });
            items.len() - 1
        }
    };
    let SolutionItem::Folder { items, .. } = &mut items[i] else { unreachable!() };
    folder_at(items, rest)
}
