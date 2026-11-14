//! Renaming a C# type also renames the file named after it (CI-5): Roslyn's
//! rename edit has no file operation, so the IDE adds one when the edit touches
//! the declaration of `Name` in `Name.cs`.

use ide_core::{FileEdit, Pos, Range, WorkspaceEdit};
use std::path::Path;

const TYPE_KEYWORDS: &[&str] = &["class", "struct", "interface", "enum", "record"];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn utf16_col(line: &str, byte: usize) -> u32 {
    u32::try_from(line[..byte].encode_utf16().count()).unwrap_or(u32::MAX)
}

/// The identifier at `pos` (a leading `@` dropped).
pub(crate) fn identifier_at(text: &str, pos: Pos) -> Option<String> {
    let line = text.lines().nth(pos.line as usize)?;
    let mut col = 0u32;
    let mut at = line.len();
    for (i, c) in line.char_indices() {
        if col >= pos.col {
            at = i;
            break;
        }
        col += u32::try_from(c.len_utf16()).unwrap_or(1);
    }
    let start = line[..at].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, _)| i);
    let end = line[at..].char_indices().find(|(_, c)| !is_word(*c)).map_or(line.len(), |(i, _)| at + i);
    (start < end).then(|| line[start..end].to_owned())
}

/// Where `text` declares the type `name`: the range of the name after `class`, `struct`,
/// `interface`, `enum` or `record`.
pub(crate) fn declaration_of(text: &str, name: &str) -> Option<Range> {
    for (number, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        let mut from = 0;
        while let Some(found) = code[from..].find(name) {
            let start = from + found;
            let end = start + name.len();
            from = end;
            let bounded = !code[..start].chars().next_back().is_some_and(|c| is_word(c) || c == '.')
                && !code[end..].chars().next().is_some_and(is_word);
            let before = code[..start].trim_end();
            let keyword = TYPE_KEYWORDS
                .iter()
                .any(|k| before.strip_suffix(k).is_some_and(|rest| !rest.chars().next_back().is_some_and(is_word)));
            // A space must separate the keyword from the name.
            if bounded && keyword && before.len() < start {
                let line_number = u32::try_from(number).unwrap_or(u32::MAX);
                return Some(Range::new(
                    Pos::new(line_number, utf16_col(line, start)),
                    Pos::new(line_number, utf16_col(line, end)),
                ));
            }
        }
    }
    None
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(is_word)
}

/// The rename of `Old.cs` to `New.cs` that belongs to `edit`, a rename of `old` to
/// `new`: when the edit changes the declaration of the type `old` in the file named
/// after it, moves no files itself, and `New.cs` is free. `read` gives a file's text.
pub(crate) fn file_rename(
    edit: &WorkspaceEdit,
    old: &str,
    new: &str,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<FileEdit> {
    if old == new || !is_identifier(old) || !is_identifier(new) {
        return None;
    }
    if edit.files.iter().any(|f| !matches!(f, FileEdit::Edit { .. })) {
        return None;
    }
    edit.files.iter().find_map(|file| {
        let FileEdit::Edit { path, edits, .. } = file else { return None };
        let is_cs = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cs"));
        if !is_cs || path.file_stem().and_then(|s| s.to_str()) != Some(old) {
            return None;
        }
        let to = path.with_file_name(format!("{new}.cs"));
        // On a case-insensitive file system a change of case finds the file itself.
        if to.exists() && !old.eq_ignore_ascii_case(new) {
            return None;
        }
        let declared = declaration_of(&read(path)?, old)?;
        let touches = |r: &Range| r.start <= declared.end && r.end >= declared.start;
        edits
            .iter()
            .any(|e| touches(&e.range))
            .then(|| FileEdit::Rename { from: path.clone(), to: to.as_path().into() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::TextEdit;

    fn edit_at(path: &str, line: u32, start: u32, end: u32, text: &str) -> FileEdit {
        FileEdit::Edit {
            path: path.into(),
            edits: vec![TextEdit {
                range: Range::new(Pos::new(line, start), Pos::new(line, end)),
                new_text: text.into(),
            }],
            version: None,
        }
    }

    const GREETER: &str = "namespace App;\n\n// class Greeter is documented here\npublic sealed class Greeter : IGreeter\n{\n    public Greeter() { }\n}\n";

    #[test]
    fn identifiers_under_the_caret() {
        let text = "var x = new Greeter();\nint @class = 1; // 😀 naïve_name";
        assert_eq!(identifier_at(text, Pos::new(0, 12)).as_deref(), Some("Greeter"));
        assert_eq!(identifier_at(text, Pos::new(0, 19)).as_deref(), Some("Greeter"), "at its end");
        assert_eq!(identifier_at(text, Pos::new(1, 5)).as_deref(), Some("class"));
        assert_eq!(identifier_at(text, Pos::new(1, 24)).as_deref(), Some("naïve_name"), "after a surrogate pair");
        assert_eq!(identifier_at(text, Pos::new(0, 6)), None);
        assert_eq!(identifier_at(text, Pos::new(9, 0)), None);
    }

    #[test]
    fn type_declarations() {
        let range = declaration_of(GREETER, "Greeter").unwrap();
        assert_eq!(range, Range::new(Pos::new(3, 20), Pos::new(3, 27)), "not the comment's");
        assert!(declaration_of("public record struct Point(int X);", "Point").is_some());
        assert!(declaration_of("enum Colour { Red }", "Colour").is_some());
        assert!(declaration_of("class Greeter2 { void Greeter() {} }", "Greeter").is_none());
        assert!(declaration_of("var subclass = Greeter.Make();", "Greeter").is_none());
        assert!(declaration_of("class Outer.Greeter", "Greeter").is_none());
    }

    #[test]
    fn a_type_rename_moves_the_file_named_after_it() {
        let read = |p: &Path| (p.file_name()? == "Greeter.cs").then(|| GREETER.to_owned());
        let edit = WorkspaceEdit {
            files: vec![
                edit_at("/p/Program.cs", 4, 8, 15, "Welcomer"),
                edit_at("/p/src/Greeter.cs", 3, 20, 27, "Welcomer"),
            ],
            ..WorkspaceEdit::default()
        };
        let renamed = file_rename(&edit, "Greeter", "Welcomer", read);
        assert_eq!(
            renamed,
            Some(FileEdit::Rename { from: "/p/src/Greeter.cs".into(), to: "/p/src/Welcomer.cs".into() })
        );
        // A minimal edit that only appends to the name still touches it.
        let appended =
            WorkspaceEdit { files: vec![edit_at("/p/Greeter.cs", 3, 27, 27, "2")], ..WorkspaceEdit::default() };
        assert!(file_rename(&appended, "Greeter", "Greeter2", read).is_some());
    }

    #[test]
    fn other_renames_leave_files_alone() {
        let read = |_: &Path| Some(GREETER.to_owned());
        let files = vec![edit_at("/p/Greeter.cs", 3, 20, 27, "Welcomer")];
        let edit = WorkspaceEdit { files: files.clone(), ..WorkspaceEdit::default() };
        assert!(file_rename(&edit, "Greeter", "Greeter", read).is_none());
        assert!(file_rename(&edit, "Greeter", "not a name", read).is_none());
        // A member that shares the file's name: the edit is elsewhere in the file.
        let member =
            WorkspaceEdit { files: vec![edit_at("/p/Greeter.cs", 5, 11, 18, "Welcomer")], ..WorkspaceEdit::default() };
        assert!(file_rename(&member, "Greeter", "Welcomer", read).is_none());
        // The type lives in a file with another name.
        let elsewhere =
            WorkspaceEdit { files: vec![edit_at("/p/Types.cs", 3, 20, 27, "Welcomer")], ..WorkspaceEdit::default() };
        assert!(file_rename(&elsewhere, "Greeter", "Welcomer", read).is_none());
        // The server already moves files.
        let mut moving = files;
        moving.push(FileEdit::Rename { from: "/p/a.cs".into(), to: "/p/b.cs".into() });
        let moving = WorkspaceEdit { files: moving, ..WorkspaceEdit::default() };
        assert!(file_rename(&moving, "Greeter", "Welcomer", read).is_none());
    }

    #[test]
    fn a_taken_file_name_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("Greeter.cs");
        std::fs::write(&from, GREETER).unwrap();
        std::fs::write(dir.path().join("Welcomer.cs"), "").unwrap();
        let edit = WorkspaceEdit {
            files: vec![edit_at(&from.to_string_lossy(), 3, 20, 27, "Welcomer")],
            ..WorkspaceEdit::default()
        };
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        assert!(file_rename(&edit, "Greeter", "Welcomer", read).is_none());
        assert!(file_rename(&edit, "Greeter", "Hello", read).is_some());
    }
}
