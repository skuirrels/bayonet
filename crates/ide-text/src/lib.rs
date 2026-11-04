//! Rope-backed text documents.
//!
//! The backend keeps the authoritative copy of every open document. The UI
//! sends CodeMirror change sets tagged with the version they were made
//! against; [`Document::apply`] rejects stale ones with `ErrorKind::OutOfSync`
//! so the UI can resync. All public coordinates are UTF-16 (see `ide-core::text`).
//!
//! Internally text is always `\n`-normalized, so `\n` is the only line break the
//! rope knows about; this matches CodeMirror (which splits on `\r\n`, `\r`, `\n`
//! and stores lines without separators) and LSP.

use ide_core::error::ErrorKind;
use ide_core::{ChangeSet, IdeError, IdeResult, Pos, Range, TextChange, TextEdit};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::borrow::Cow;

pub use ropey;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// The dominant line ending of `text`; ties and text without any line
    /// break resolve to [`LineEnding::Lf`].
    pub fn detect(text: &str) -> Self {
        let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => lf += 1,
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        i += 1;
                    } else {
                        cr += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if lf >= crlf && lf >= cr {
            Self::Lf
        } else if crlf >= cr {
            Self::CrLf
        } else {
            Self::Cr
        }
    }
}

/// Converts `\r\n` and lone `\r` to `\n`. Borrows when there is nothing to do.
pub fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// An incremental change in LSP form, produced while applying a [`ChangeSet`]
/// so `ide-lsp` can forward `textDocument/didChange` without re-diffing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedChange {
    /// Range in the document *as it was immediately before this change*.
    pub range: Range,
    pub text: String,
}

/// A point in byte coordinates: 0-based row and byte column within the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BytePoint {
    pub row: usize,
    pub col: usize,
}

/// One change in byte coordinates of the `\n`-normalized text, in the same
/// sequential order as [`AppliedChange`]; maps 1:1 onto tree-sitter's `InputEdit`
/// so syntax trees can be re-parsed incrementally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: BytePoint,
    pub old_end: BytePoint,
    pub new_end: BytePoint,
}

/// Text normalized to `\n` line endings internally; the original line ending is
/// remembered and restored on save.
#[derive(Clone, Debug)]
pub struct Document {
    rope: ropey::Rope,
    version: u32,
    line_ending: LineEnding,
}

impl Document {
    /// Builds a document from text with any line endings; detects and normalizes them.
    pub fn new(text: &str) -> Self {
        let line_ending = LineEnding::detect(text);
        let rope = ropey::Rope::from_str(&normalize_line_endings(text));
        Self { rope, version: 0, line_ending }
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, le: LineEnding) {
        self.line_ending = le;
    }

    /// Normalized text (`\n`).
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Text with the document's original line endings, for writing to disk.
    pub fn text_for_save(&self) -> String {
        if self.line_ending == LineEnding::Lf {
            return self.text();
        }
        let sep = self.line_ending.as_str();
        let mut out = String::with_capacity(self.rope.len_bytes() + self.rope.len_lines());
        for chunk in self.rope.chunks() {
            let mut rest = chunk;
            while let Some(i) = rest.find('\n') {
                out.push_str(&rest[..i]);
                out.push_str(sep);
                rest = &rest[i + 1..];
            }
            out.push_str(rest);
        }
        out
    }

    pub fn len_utf16(&self) -> u32 {
        self.rope.len_utf16_cu() as u32
    }

    pub fn line_count(&self) -> u32 {
        self.rope.len_lines() as u32
    }

    /// Line text without the trailing newline.
    pub fn line(&self, line: u32) -> Option<String> {
        let slice = self.rope.get_line(line as usize)?;
        let mut s = slice.to_string();
        if s.ends_with('\n') {
            s.pop();
        }
        Some(s)
    }

    /// Text between two UTF-16 offsets (clamped, rounded down to char boundaries).
    pub fn slice(&self, from: u32, to: u32) -> String {
        let a = self.utf16_to_char(from);
        let b = self.utf16_to_char(to).max(a);
        self.rope.slice(a..b).to_string()
    }

    /// Applies a change set made against `base_version`; bumps the version.
    ///
    /// Changes must be sorted and non-overlapping, in UTF-16 offsets of the
    /// document before the change set. The result lists the same changes in
    /// sequential (LSP `didChange`) form; no-op changes are omitted. The
    /// document is left untouched when an error is returned.
    pub fn apply(&mut self, base_version: u32, changes: &ChangeSet) -> IdeResult<Vec<AppliedChange>> {
        self.apply_detailed(base_version, changes).map(|(applied, _)| applied)
    }

    /// [`Document::apply`] that also reports every change in byte coordinates.
    pub fn apply_detailed(
        &mut self,
        base_version: u32,
        changes: &ChangeSet,
    ) -> IdeResult<(Vec<AppliedChange>, Vec<ByteEdit>)> {
        if base_version != self.version {
            return Err(IdeError::new(
                ErrorKind::OutOfSync,
                format!("change made against version {base_version}, document is at {}", self.version),
            ));
        }
        let len = self.len_utf16();
        let mut prev_to = 0u32;
        for (i, c) in changes.changes.iter().enumerate() {
            if c.from > c.to || c.to > len {
                return Err(IdeError::invalid(format!(
                    "change {i} [{}, {}) is out of range (document length {len})",
                    c.from, c.to
                )));
            }
            if c.from < prev_to {
                return Err(IdeError::invalid(format!("change {i} overlaps or precedes the previous change")));
            }
            prev_to = c.to;
        }

        // Resolve against the original rope before mutating anything.
        let resolved: Vec<(usize, usize, Cow<'_, str>)> = changes
            .changes
            .iter()
            .map(|c| {
                let from = self.utf16_to_char(c.from);
                let to = self.utf16_to_char(c.to).max(from);
                (from, to, normalize_line_endings(&c.insert))
            })
            .collect();

        let mut applied = Vec::with_capacity(resolved.len());
        let mut bytes = Vec::with_capacity(resolved.len());
        let mut delta: isize = 0;
        for (from, to, insert) in resolved {
            if from == to && insert.is_empty() {
                continue;
            }
            let from = (from as isize + delta) as usize;
            let to = (to as isize + delta) as usize;
            let range = Range::new(self.char_to_pos(from), self.char_to_pos(to));
            let (start_byte, start) = self.char_to_byte_point(from);
            let (old_end_byte, old_end) = self.char_to_byte_point(to);
            let new_end = match insert.rfind('\n') {
                Some(i) => BytePoint { row: start.row + insert.matches('\n').count(), col: insert.len() - i - 1 },
                None => BytePoint { row: start.row, col: start.col + insert.len() },
            };
            bytes.push(ByteEdit {
                start_byte,
                old_end_byte,
                new_end_byte: start_byte + insert.len(),
                start,
                old_end,
                new_end,
            });
            if to > from {
                self.rope.remove(from..to);
            }
            if !insert.is_empty() {
                self.rope.insert(from, &insert);
            }
            delta += insert.chars().count() as isize - (to - from) as isize;
            applied.push(AppliedChange { range, text: insert.into_owned() });
        }
        self.version = self.version.wrapping_add(1);
        Ok((applied, bytes))
    }

    /// Applies line/column edits (formatting, refactoring); returns the equivalent
    /// change set so it can be pushed to the UI as `IdeEvent::DocPatched`.
    ///
    /// Edit ranges refer to the current document and may come in any order;
    /// inserts at the same position keep their relative order. Overlapping
    /// edits are rejected. Bumps the version by one.
    pub fn apply_edits(&mut self, edits: &[TextEdit]) -> IdeResult<(ChangeSet, Vec<AppliedChange>)> {
        self.apply_edits_detailed(edits).map(|(cs, applied, _)| (cs, applied))
    }

    /// [`Document::apply_edits`] that also reports every change in byte coordinates.
    pub fn apply_edits_detailed(
        &mut self,
        edits: &[TextEdit],
    ) -> IdeResult<(ChangeSet, Vec<AppliedChange>, Vec<ByteEdit>)> {
        let mut items = Vec::with_capacity(edits.len());
        for (i, e) in edits.iter().enumerate() {
            if e.range.start > e.range.end {
                return Err(IdeError::invalid(format!("edit {i} has an inverted range")));
            }
            let from = self.pos_to_offset(e.range.start);
            let to = self.pos_to_offset(e.range.end);
            items.push((from, to, i));
        }
        items.sort_unstable_by_key(|&(from, to, i)| (from, to, i));
        let mut changes = Vec::with_capacity(items.len());
        let mut prev_to = 0u32;
        for &(from, to, i) in &items {
            if from < prev_to {
                return Err(IdeError::invalid(format!("edit {i} overlaps another edit")));
            }
            prev_to = to;
            let insert = normalize_line_endings(&edits[i].new_text);
            if from == to && insert.is_empty() {
                continue;
            }
            changes.push(TextChange { from, to, insert: insert.into_owned() });
        }
        let cs = ChangeSet { changes };
        let (applied, bytes) = self.apply_detailed(self.version, &cs)?;
        Ok((cs, applied, bytes))
    }

    /// Replaces the whole content (reload from disk), returning a minimal change set.
    ///
    /// The version is bumped only when the text actually changes (i.e. when the
    /// returned change set is non-empty). Line endings of `text` are normalized;
    /// the document's [`LineEnding`] is not changed.
    pub fn replace_all(&mut self, text: &str) -> ChangeSet {
        let cs = self.diff(text);
        if !cs.changes.is_empty() {
            self.apply(self.version, &cs).expect("diff produces a valid change set");
        }
        cs
    }

    /// The minimal single-change (common prefix/suffix) change set turning the
    /// current text into `text` (after line-ending normalization). Empty when equal.
    pub fn diff(&self, text: &str) -> ChangeSet {
        let new = normalize_line_endings(text);
        let old_chars = self.rope.len_chars();

        let mut prefix_chars = 0usize;
        let mut prefix_bytes = 0usize;
        for (a, b) in self.rope.chars().zip(new.chars()) {
            if a != b {
                break;
            }
            prefix_chars += 1;
            prefix_bytes += b.len_utf8();
        }
        let new_rest = &new[prefix_bytes..];
        let new_rest_chars = new_rest.chars().count();
        if prefix_chars == old_chars && new_rest_chars == 0 {
            return ChangeSet::default();
        }

        let max_suffix = (old_chars - prefix_chars).min(new_rest_chars);
        let mut suffix_chars = 0usize;
        let mut suffix_bytes = 0usize;
        for (a, b) in self.rope.chars_at(old_chars).reversed().zip(new_rest.chars().rev()).take(max_suffix) {
            if a != b {
                break;
            }
            suffix_chars += 1;
            suffix_bytes += b.len_utf8();
        }

        let from = self.rope.char_to_utf16_cu(prefix_chars) as u32;
        let to = self.rope.char_to_utf16_cu(old_chars - suffix_chars) as u32;
        let insert = new_rest[..new_rest.len() - suffix_bytes].to_owned();
        ChangeSet { changes: vec![TextChange { from, to, insert }] }
    }

    // ---- coordinate conversion (UTF-16 offset ⇄ Pos ⇄ byte offset) ----
    //
    // All conversions clamp out-of-range input to the document and round
    // positions inside a surrogate pair / multi-byte char down to its start.

    pub fn offset_to_pos(&self, offset: u32) -> Pos {
        self.char_to_pos(self.utf16_to_char(offset))
    }
    pub fn pos_to_offset(&self, pos: Pos) -> u32 {
        self.rope.char_to_utf16_cu(self.pos_to_char(pos)) as u32
    }
    pub fn byte_to_pos(&self, byte: usize) -> Pos {
        let byte = byte.min(self.rope.len_bytes());
        self.char_to_pos(self.rope.byte_to_char(byte))
    }
    pub fn pos_to_byte(&self, pos: Pos) -> usize {
        self.rope.char_to_byte(self.pos_to_char(pos))
    }
    /// UTF-16 offset → byte offset.
    pub fn offset_to_byte(&self, offset: u32) -> usize {
        self.rope.char_to_byte(self.utf16_to_char(offset))
    }
    /// Byte offset → UTF-16 offset.
    pub fn byte_to_offset(&self, byte: usize) -> u32 {
        let byte = byte.min(self.rope.len_bytes());
        self.rope.char_to_utf16_cu(self.rope.byte_to_char(byte)) as u32
    }
    pub fn rope(&self) -> &ropey::Rope {
        &self.rope
    }

    fn utf16_to_char(&self, offset: u32) -> usize {
        let offset = (offset as usize).min(self.rope.len_utf16_cu());
        self.rope.utf16_cu_to_char(offset)
    }

    fn char_to_byte_point(&self, ch: usize) -> (usize, BytePoint) {
        let byte = self.rope.char_to_byte(ch);
        let row = self.rope.char_to_line(ch);
        (byte, BytePoint { row, col: byte - self.rope.line_to_byte(row) })
    }

    fn char_to_pos(&self, ch: usize) -> Pos {
        let line = self.rope.char_to_line(ch);
        let line_start = self.rope.line_to_char(line);
        let col = self.rope.char_to_utf16_cu(ch) - self.rope.char_to_utf16_cu(line_start);
        Pos::new(line as u32, col as u32)
    }

    fn pos_to_char(&self, pos: Pos) -> usize {
        let line = pos.line as usize;
        if line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        let start = self.rope.line_to_char(line);
        let end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_char(line + 1) - 1 // exclude the `\n`
        } else {
            self.rope.len_chars()
        };
        let start16 = self.rope.char_to_utf16_cu(start);
        let end16 = self.rope.char_to_utf16_cu(end);
        let target = (start16 + pos.col as usize).min(end16);
        self.rope.utf16_cu_to_char(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(from: u32, to: u32, insert: &str) -> TextChange {
        TextChange { from, to, insert: insert.into() }
    }

    fn cs(changes: Vec<TextChange>) -> ChangeSet {
        ChangeSet { changes }
    }

    #[test]
    fn only_newline_breaks_lines() {
        // Guards against ropey's `unicode_lines` / `cr_lines` being enabled by
        // feature unification, which would desync line numbers from the UI.
        let doc = Document::new("a\u{0B}b\u{0C}c\u{85}d\u{2028}e\u{2029}f");
        assert_eq!(doc.line_count(), 1);
    }

    #[test]
    fn line_ending_detection_and_normalization() {
        assert_eq!(LineEnding::detect(""), LineEnding::Lf);
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\rb\rc\r\n"), LineEnding::Cr);
        assert_eq!(LineEnding::detect("a\r\nb\n"), LineEnding::Lf);
        assert_eq!(normalize_line_endings("a\r\nb\rc\nd\r"), "a\nb\nc\nd\n");
        assert!(matches!(normalize_line_endings("a\nb"), Cow::Borrowed(_)));

        let doc = Document::new("one\r\ntwo\r\n");
        assert_eq!(doc.text(), "one\ntwo\n");
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        assert_eq!(doc.text_for_save(), "one\r\ntwo\r\n");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line(1).as_deref(), Some("two"));
        assert_eq!(doc.line(2).as_deref(), Some(""));
        assert_eq!(doc.line(3), None);

        let mut doc = Document::new("x\ry");
        assert_eq!(doc.text_for_save(), "x\ry");
        doc.set_line_ending(LineEnding::Lf);
        assert_eq!(doc.text_for_save(), "x\ny");
    }

    #[test]
    fn empty_document() {
        let doc = Document::new("");
        assert_eq!(doc.len_utf16(), 0);
        assert_eq!(doc.line_count(), 1);
        assert_eq!(doc.line(0).as_deref(), Some(""));
        assert_eq!(doc.offset_to_pos(5), Pos::new(0, 0));
        assert_eq!(doc.pos_to_offset(Pos::new(3, 3)), 0);
        assert_eq!(doc.byte_to_pos(9), Pos::new(0, 0));
        assert_eq!(doc.pos_to_byte(Pos::new(1, 0)), 0);
    }

    #[test]
    fn coordinates_with_surrogates() {
        // "a😀b\nc" : 😀 is 2 UTF-16 units, 4 bytes.
        let doc = Document::new("a😀b\nc");
        assert_eq!(doc.len_utf16(), 6);
        assert_eq!(doc.offset_to_pos(3), Pos::new(0, 3));
        assert_eq!(doc.offset_to_pos(4), Pos::new(0, 4));
        assert_eq!(doc.offset_to_pos(5), Pos::new(1, 0));
        assert_eq!(doc.offset_to_pos(2), Pos::new(0, 1), "mid-surrogate rounds down");
        assert_eq!(doc.pos_to_offset(Pos::new(0, 99)), 4, "column clamps to line end");
        assert_eq!(doc.pos_to_offset(Pos::new(9, 0)), 6, "line clamps to document end");
        assert_eq!(doc.pos_to_byte(Pos::new(0, 3)), 5);
        assert_eq!(doc.byte_to_pos(5), Pos::new(0, 3));
        assert_eq!(doc.byte_to_pos(3), Pos::new(0, 1), "mid-char byte rounds down");
        assert_eq!(doc.offset_to_byte(3), 5);
        assert_eq!(doc.byte_to_offset(5), 3);
        assert_eq!(doc.slice(1, 3), "😀");
    }

    #[test]
    fn apply_change_set_returns_sequential_changes() {
        let mut doc = Document::new("hello\nworld\n");
        let applied = doc.apply(0, &cs(vec![tc(0, 5, "HI"), tc(6, 6, ">> "), tc(11, 12, "")])).unwrap();
        assert_eq!(doc.text(), "HI\n>> world");
        assert_eq!(doc.version(), 1);
        assert_eq!(
            applied,
            vec![
                AppliedChange { range: Range::new(Pos::new(0, 0), Pos::new(0, 5)), text: "HI".into() },
                AppliedChange { range: Range::point(Pos::new(1, 0)), text: ">> ".into() },
                AppliedChange { range: Range::new(Pos::new(1, 8), Pos::new(2, 0)), text: String::new() },
            ]
        );
    }

    #[test]
    fn apply_rejects_bad_input() {
        let mut doc = Document::new("abc");
        let err = doc.apply(3, &cs(vec![tc(0, 0, "x")])).unwrap_err();
        assert_eq!(err.kind, ErrorKind::OutOfSync);
        let err = doc.apply(0, &cs(vec![tc(0, 4, "x")])).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        let err = doc.apply(0, &cs(vec![tc(2, 1, "x")])).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        let err = doc.apply(0, &cs(vec![tc(1, 2, "x"), tc(0, 1, "y")])).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert_eq!(doc.text(), "abc");
        assert_eq!(doc.version(), 0);
        // Adjacent changes and repeated inserts at one position are fine.
        doc.apply(0, &cs(vec![tc(1, 1, "x"), tc(1, 1, "y"), tc(1, 2, "Z")])).unwrap();
        assert_eq!(doc.text(), "axyZc");
    }

    #[test]
    fn inserted_crlf_is_normalized() {
        let mut doc = Document::new("ab");
        let applied = doc.apply(0, &cs(vec![tc(1, 1, "\r\n")])).unwrap();
        assert_eq!(doc.text(), "a\nb");
        assert_eq!(applied[0].text, "\n");
    }

    #[test]
    fn apply_edits_unsorted_and_overlapping() {
        let mut doc = Document::new("fn main() {\n  x\n}\n");
        let edits = vec![
            TextEdit { range: Range::new(Pos::new(1, 0), Pos::new(1, 2)), new_text: "    ".into() },
            TextEdit { range: Range::new(Pos::new(0, 3), Pos::new(0, 7)), new_text: "start".into() },
        ];
        let (set, applied) = doc.apply_edits(&edits).unwrap();
        assert_eq!(doc.text(), "fn start() {\n    x\n}\n");
        assert_eq!(set.changes, vec![tc(3, 7, "start"), tc(12, 14, "    ")]);
        assert_eq!(applied[1].range, Range::new(Pos::new(1, 0), Pos::new(1, 2)));
        assert_eq!(doc.version(), 1);

        let overlapping = vec![
            TextEdit { range: Range::new(Pos::new(0, 0), Pos::new(0, 4)), new_text: "a".into() },
            TextEdit { range: Range::new(Pos::new(0, 2), Pos::new(0, 6)), new_text: "b".into() },
        ];
        assert_eq!(doc.apply_edits(&overlapping).unwrap_err().kind, ErrorKind::InvalidInput);
        assert_eq!(doc.version(), 1);

        // Same-position inserts keep their array order.
        let mut doc = Document::new("ab");
        let inserts = vec![
            TextEdit { range: Range::point(Pos::new(0, 1)), new_text: "1".into() },
            TextEdit { range: Range::point(Pos::new(0, 1)), new_text: "2".into() },
        ];
        doc.apply_edits(&inserts).unwrap();
        assert_eq!(doc.text(), "a12b");
    }

    #[test]
    fn replace_all_minimal_diff() {
        let mut doc = Document::new("let a = 1;\nlet b = 2;\n");
        let set = doc.replace_all("let a = 1;\nlet bb = 2;\r\n");
        assert_eq!(set.changes, vec![tc(16, 16, "b")]);
        assert_eq!(doc.text(), "let a = 1;\nlet bb = 2;\n");
        assert_eq!(doc.version(), 1);
        assert!(doc.replace_all("let a = 1;\nlet bb = 2;\n").changes.is_empty());
        assert_eq!(doc.version(), 1);

        // Repeated characters: prefix and suffix must not overlap.
        let mut doc = Document::new("aaa");
        let set = doc.replace_all("aa");
        assert_eq!(doc.text(), "aa");
        assert_eq!(set.changes, vec![tc(2, 3, "")]);
        let mut doc = Document::new("😀😀");
        let set = doc.replace_all("😀x😀");
        assert_eq!(set.changes, vec![tc(2, 2, "x")]);
        assert_eq!(doc.text(), "😀x😀");
    }
}
