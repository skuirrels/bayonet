//! Change Signature for C# (CI-6): reorders, removes and adds the parameters of
//! a method, constructor, local function or delegate, and updates its call
//! sites to match.
//!
//! Roslyn's language server does not offer Change Signature, so the IDE does it
//! over syntax trees: the caller finds the call sites with the server's
//! references and hands them in with each file's text. Each site is rewritten
//! on its own; arguments move with their parameters, omitted optional
//! arguments and everything after them become named arguments, and new
//! parameters get the caller value the user gave. A site that cannot be
//! rewritten safely (a method group, a `params` expansion after a gap, an
//! unknown named argument) is left alone and reported.

use crate::lines::Lines;
use ide_core::{FileEdit, IpcPath, LanguageId, Pos, Range, TextEdit, WorkspaceEdit};
use ide_language::Syntax;
use serde::{Deserialize, Serialize};
use specta::Type;
use tree_sitter::Node;

const DECLARATIONS: &[&str] =
    &["method_declaration", "constructor_declaration", "local_function_statement", "delegate_declaration"];
const TYPES: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "record_declaration",
    "record_struct_declaration",
    "interface_declaration",
];
const MODIFIERS: &[&str] = &["this", "params", "ref", "out", "in", "scoped", "readonly"];

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    /// The whole declaration, e.g. `int count = 0`.
    pub text: String,
    /// As written, with any `@`.
    pub name: String,
    /// `this`, `params`, `ref`, `out`, `in`, `scoped`, `readonly`.
    pub modifiers: Vec<String>,
    pub has_default: bool,
}

/// The signature under the caret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MethodSignature {
    pub name: String,
    /// `method`, `constructor`, `localFunction` or `delegate`.
    pub kind: String,
    /// The enclosing type's name.
    pub container: Option<String>,
    pub parameters: Vec<Parameter>,
    /// The first parameter is `this` (an extension method).
    pub extension: bool,
    /// The declared name, where references are looked up.
    pub name_range: Range,
    /// The declaration as written up to its body, for the dialog's preview.
    pub header: String,
}

/// A parameter of the new signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewParameter {
    /// Index of the existing parameter it is; `None` for a new one.
    pub from: Option<u32>,
    /// The declaration, e.g. `CancellationToken token = default`.
    pub text: String,
    /// What callers pass for a new parameter; empty or `None` omits it when it has a default.
    pub value: Option<String>,
}

/// A file with call sites: its text and the reference locations in it.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSites {
    pub path: IpcPath,
    pub text: String,
    pub locations: Vec<Range>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedSite {
    pub path: IpcPath,
    pub range: Range,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignatureRewrite {
    pub edit: WorkspaceEdit,
    /// Declarations rewritten (the method and any overrides or implementations).
    pub declarations: u32,
    pub calls: u32,
    pub skipped: Vec<SkippedSite>,
}

fn csharp() -> LanguageId {
    LanguageId::new("csharp")
}

/// The method, constructor, local function or delegate declared around `pos`.
pub fn signature_at(syntax: &Syntax, text: &str, pos: Pos) -> Option<MethodSignature> {
    let lines = Lines::new(text);
    let byte = lines.byte(pos);
    syntax
        .with_parsed(&csharp(), text, &|| false, |parsed| {
            let root = parsed.tree.root_node();
            let mut node = root.descendant_for_byte_range(byte, byte)?;
            loop {
                if DECLARATIONS.contains(&node.kind()) {
                    // Inside a body means a local function or lambda further in, or the
                    // method itself when nothing closer is declared.
                    return declaration(node, text, &lines);
                }
                node = node.parent()?;
            }
        })
        .ok()
        .flatten()
}

fn declaration(node: Node, text: &str, lines: &Lines) -> Option<MethodSignature> {
    let name = node.child_by_field_name("name")?;
    let list = node.child_by_field_name("parameters")?;
    let parameters = split_list(text, list).into_iter().map(|(s, e)| parameter(&text[s..e])).collect::<Vec<_>>();
    let kind = match node.kind() {
        "constructor_declaration" => "constructor",
        "local_function_statement" => "localFunction",
        "delegate_declaration" => "delegate",
        _ => "method",
    };
    let mut container = None;
    let mut up = node.parent();
    while let Some(p) = up {
        if TYPES.contains(&p.kind()) {
            container = p.child_by_field_name("name").map(|n| text[n.byte_range()].to_string());
            break;
        }
        up = p.parent();
    }
    let header_end = node.child_by_field_name("body").map_or(node.end_byte(), |b| b.start_byte());
    let header = text[node.start_byte()..header_end].trim_end().trim_end_matches(';').trim_end();
    Some(MethodSignature {
        name: text[name.byte_range()].to_string(),
        kind: kind.into(),
        container,
        extension: parameters.first().is_some_and(|p| p.modifiers.iter().any(|m| m == "this")),
        parameters,
        name_range: lines.range(name.start_byte(), name.end_byte()),
        header: header.split_whitespace().collect::<Vec<_>>().join(" "),
    })
}

/// A parameter from its declaration text.
pub fn parameter(text: &str) -> Parameter {
    let text = text.trim();
    let (head, has_default) = match top_level(text, b'=') {
        Some(i) => (text[..i].trim_end(), true),
        None => (text, false),
    };
    // Attributes first, then modifiers, the type and the name.
    let mut rest = head;
    while rest.starts_with('[') {
        match matching(rest, 0) {
            Some(end) => rest = rest[end + 1..].trim_start(),
            None => break,
        }
    }
    let mut modifiers = Vec::new();
    loop {
        let word: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let next = rest[word.len()..].chars().next();
        if MODIFIERS.contains(&word.as_str()) && next.is_some_and(char::is_whitespace) {
            modifiers.push(word.clone());
            rest = rest[word.len()..].trim_start();
        } else {
            break;
        }
    }
    let name: String = {
        let tail: Vec<char> =
            rest.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '@').collect();
        tail.into_iter().rev().collect()
    };
    Parameter { text: text.to_string(), name, modifiers, has_default }
}

/// Byte index of `target` outside brackets and literals.
fn top_level(text: &str, target: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_literal(bytes, i),
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            // `==`, `=>`, `<=` and `>=` never appear at the top of a parameter or argument.
            b if b == target && depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the bracket closing the one at `open`.
fn matching(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_literal(bytes, i),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the quote closing the literal opened at `i`.
fn skip_literal(bytes: &[u8], i: usize) -> usize {
    let quote = bytes[i];
    let verbatim = i > 0 && bytes[i - 1] == b'@';
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' && !verbatim {
            j += 2;
            continue;
        }
        if bytes[j] == quote {
            if verbatim && bytes.get(j + 1) == Some(&quote) {
                j += 2;
                continue;
            }
            return j;
        }
        j += 1;
    }
    bytes.len()
}

/// The items of a parenthesised list node, as trimmed byte ranges of `text`.
fn split_list(text: &str, list: Node) -> Vec<(usize, usize)> {
    let (start, end) = inner(list);
    let inside = &text[start..end];
    let bytes = inside.as_bytes();
    let mut out = Vec::new();
    let (mut from, mut i) = (0usize, 0usize);
    let push = |s: usize, e: usize, out: &mut Vec<(usize, usize)>| {
        let item = &inside[s..e];
        let lead = item.len() - item.trim_start().len();
        let trail = item.len() - item.trim_end().len();
        if lead + trail < item.len() {
            out.push((start + s + lead, start + e - trail));
        }
    };
    // Open brackets; `<` only counts after an identifier (`List<int>`), and
    // `>` only closes one (`x => x > 1` does not).
    let mut stack: Vec<u8> = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_literal(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = inside[i + 2..].find("*/").map_or(bytes.len(), |j| i + 2 + j + 2);
                continue;
            }
            b'(' | b'[' | b'{' => stack.push(bytes[i]),
            b'<' if is_generic_open(bytes, i) => stack.push(b'<'),
            b'>' if stack.last() == Some(&b'<') => {
                stack.pop();
            }
            b')' | b']' | b'}' => {
                // Unclosed `<` were comparisons after all.
                while stack.last() == Some(&b'<') {
                    stack.pop();
                }
                stack.pop();
            }
            b',' if stack.is_empty() => {
                push(from, i, &mut out);
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push(from, bytes.len(), &mut out);
    out
}

/// A `<` right after an identifier starts type arguments; elsewhere it compares.
fn is_generic_open(bytes: &[u8], i: usize) -> bool {
    i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') && {
        // `a < b` has spaces around it in practice; `List<int>` does not.
        bytes.get(i + 1).is_some_and(|b| !b.is_ascii_whitespace())
    }
}

/// The byte range inside a list's brackets.
fn inner(list: Node) -> (usize, usize) {
    let (s, e) = (list.start_byte(), list.end_byte());
    (s + 1, e.saturating_sub(1).max(s + 1))
}

/// The text between the first two items, else `, `.
fn separator(text: &str, items: &[(usize, usize)]) -> String {
    match items {
        [a, b, ..] => text[a.1..b.0].to_string(),
        _ => ", ".into(),
    }
}

enum Site<'t> {
    Declaration(Node<'t>),
    Call { args: Node<'t>, receiver: Option<String> },
    Skip(&'static str),
}

/// What the reference at `byte` is.
fn site_at<'t>(root: Node<'t>, byte: usize, text: &str) -> Site<'t> {
    let Some(mut node) = root.descendant_for_byte_range(byte, byte) else {
        return Site::Skip("no syntax at the reference");
    };
    let before = |n: Option<Node>| n.is_none_or(|a| byte < a.start_byte());
    for _ in 0..8 {
        let kind = node.kind();
        if DECLARATIONS.contains(&kind) {
            let on_name = node.child_by_field_name("name").is_some_and(|n| n.byte_range().contains(&byte));
            return match node.child_by_field_name("parameters") {
                Some(p) if on_name => Site::Declaration(p),
                _ => Site::Skip("not a call"),
            };
        }
        match kind {
            "invocation_expression" => {
                let args = node.child_by_field_name("arguments");
                let function = node.child_by_field_name("function");
                return match (args, function) {
                    (Some(args), Some(f)) if before(Some(args)) => Site::Call { args, receiver: receiver(f, text) },
                    _ => Site::Skip("a method group, not a call"),
                };
            }
            "object_creation_expression" | "implicit_object_creation_expression" | "constructor_initializer" => {
                let args = node.child_by_field_name("arguments").or_else(|| {
                    (0..node.named_child_count())
                        .filter_map(|i| node.named_child(i))
                        .find(|c| c.kind() == "argument_list")
                });
                return match args {
                    Some(args) if before(Some(args)) => Site::Call { args, receiver: None },
                    Some(_) => Site::Skip("not a call"),
                    None => Site::Skip("an object initializer without an argument list"),
                };
            }
            k if k.ends_with("_statement") || k == "block" || k == "declaration_list" || k == "argument" => {
                return Site::Skip("a method group, not a call");
            }
            _ => {}
        }
        match node.parent() {
            Some(p) => node = p,
            None => break,
        }
    }
    Site::Skip("not a call")
}

/// The receiver text of `x.Name(...)` or `x?.Name(...)`.
fn receiver(function: Node, text: &str) -> Option<String> {
    match function.kind() {
        "member_access_expression" => {
            let name = function.child_by_field_name("name")?;
            let head = text[function.start_byte()..name.start_byte()].trim_end();
            Some(head.trim_end_matches('.').trim_end_matches("?").trim().to_string())
        }
        "conditional_access_expression" => {
            function.child_by_field_name("condition").map(|c| text[c.byte_range()].trim().to_string())
        }
        _ => None,
    }
}

struct Arg {
    /// As written, with any `ref` / `out` / `in`.
    text: String,
    name: Option<String>,
    /// Without the name.
    value: String,
}

fn arguments(text: &str, list: Node) -> Vec<Arg> {
    split_list(text, list)
        .into_iter()
        .map(|(s, e)| {
            let item = &text[s..e];
            // `name: value`, but not `cond ? a : b` or `global::X`.
            let named = item
                .find(':')
                .filter(|&i| {
                    let head = item[..i].trim();
                    !head.is_empty()
                        && head.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '@')
                        && item.as_bytes().get(i + 1) != Some(&b':')
                })
                .map(|i| (item[..i].trim().trim_start_matches('@').to_string(), item[i + 1..].trim().to_string()));
            match named {
                Some((name, value)) => Arg { text: item.to_string(), name: Some(name), value },
                None => Arg { text: item.to_string(), name: None, value: item.to_string() },
            }
        })
        .collect()
}

/// The new argument list for a call; `Err` names why it is left alone.
fn rewrite_arguments(
    sig: &MethodSignature,
    change: &[(NewParameter, Parameter)],
    args: &[Arg],
    receiver: bool,
) -> Result<Vec<String>, &'static str> {
    let old = &sig.parameters;
    let mut assigned: Vec<Vec<&Arg>> = vec![Vec::new(); old.len()];
    let mut next = usize::from(receiver);
    for arg in args {
        let index = match &arg.name {
            Some(name) => old
                .iter()
                .position(|p| p.name.trim_start_matches('@') == name)
                .ok_or("an argument names an unknown parameter")?,
            None => {
                let last_params = old.last().is_some_and(|p| p.modifiers.iter().any(|m| m == "params"));
                if next >= old.len() {
                    if !last_params {
                        return Err("more arguments than parameters");
                    }
                    old.len() - 1
                } else {
                    next
                }
            }
        };
        if arg.name.is_none() && !old[index].modifiers.iter().any(|m| m == "params") {
            next += 1;
        }
        assigned[index].push(arg);
    }
    let mut out = Vec::new();
    let mut named = false;
    for (i, (np, new)) in change.iter().enumerate() {
        if receiver && i == 0 {
            continue;
        }
        let name = &new.name;
        match np.from {
            Some(from) => {
                let given = assigned.get(from as usize).ok_or("a parameter index is out of range")?;
                match given.as_slice() {
                    [] => named = true,
                    [arg] => {
                        if arg.name.is_some() || named {
                            named = true;
                            out.push(format!("{name}: {}", arg.value));
                        } else {
                            out.push(arg.text.clone());
                        }
                    }
                    many => {
                        if named {
                            return Err("a params argument list after an omitted argument");
                        }
                        out.extend(many.iter().map(|a| a.text.clone()));
                    }
                }
            }
            None => match np.value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                Some(value) if named => out.push(format!("{name}: {value}")),
                Some(value) => out.push(value.to_string()),
                None if new.has_default => named = true,
                None => return Err("no value for a new parameter"),
            },
        }
    }
    Ok(out)
}

/// One replacement in a file, by byte range.
struct Splice {
    start: usize,
    end: usize,
    text: String,
}

/// Applies `inside` (all within `start..end`) to that slice of `text`.
fn spliced(text: &str, start: usize, end: usize, inside: &[Splice]) -> String {
    let mut out = String::new();
    let mut at = start;
    for s in inside {
        out.push_str(&text[at..s.start]);
        out.push_str(&s.text);
        at = s.end;
    }
    out.push_str(&text[at..end]);
    out
}

/// Rewrites the declarations and calls at `files`' locations for `change`.
pub fn change_signature(
    syntax: &Syntax,
    sig: &MethodSignature,
    change: &[NewParameter],
    files: &[SignatureSites],
) -> Result<SignatureRewrite, String> {
    let resolved: Vec<(NewParameter, Parameter)> = change.iter().map(|p| (p.clone(), parameter(&p.text))).collect();
    for (np, p) in &resolved {
        if p.name.is_empty() {
            return Err(format!("'{}' has no parameter name", np.text));
        }
        if let Some(from) = np.from {
            let old = sig.parameters.get(from as usize).ok_or("a parameter index is out of range")?;
            if old.name != p.name {
                return Err(format!("Rename '{}' with Rename, not Change Signature", old.name));
            }
        } else if p.modifiers.iter().all(|m| m != "params")
            && !p.has_default
            && np.value.as_deref().is_none_or(|v| v.trim().is_empty())
        {
            return Err(format!("Give a value for callers to pass for '{}'", p.name));
        }
    }
    let mut names = std::collections::HashSet::new();
    if let Some((_, dup)) = resolved.iter().find(|(_, p)| !names.insert(p.name.clone())) {
        return Err(format!("'{}' is declared twice", dup.name));
    }
    if sig.extension && resolved.first().and_then(|(np, _)| np.from) != Some(0) {
        return Err("The 'this' parameter of an extension method stays first".into());
    }
    if let Some(i) = resolved.iter().position(|(_, p)| p.modifiers.iter().any(|m| m == "params"))
        && i + 1 != resolved.len()
    {
        return Err("A params parameter must be last".into());
    }

    let mut out = SignatureRewrite::default();
    for file in files {
        let lines = Lines::new(&file.text);
        let text = file.text.as_str();
        let edits = syntax
            .with_parsed(&csharp(), text, &|| false, |parsed| {
                let root = parsed.tree.root_node();
                // (list node range, replacement of its inside), innermost first.
                let mut splices: Vec<Splice> = Vec::new();
                let mut seen = std::collections::HashSet::new();
                let mut sites: Vec<(Node, Option<Option<String>>)> = Vec::new();
                for range in &file.locations {
                    match site_at(root, lines.byte(range.start), text) {
                        Site::Declaration(list) => {
                            if seen.insert(list.id()) {
                                sites.push((list, None));
                            }
                        }
                        Site::Call { args, receiver } => {
                            if seen.insert(args.id()) {
                                sites.push((args, Some(receiver)));
                            }
                        }
                        Site::Skip(reason) => out.skipped.push(SkippedSite {
                            path: file.path.clone(),
                            range: *range,
                            reason: reason.into(),
                        }),
                    }
                }
                // Inner lists first so an outer call's arguments carry their rewrite.
                sites.sort_by_key(|(n, _)| n.end_byte() - n.start_byte());
                for (list, call) in sites {
                    let (start, end) = inner(list);
                    let items = split_list(text, list);
                    let sep = separator(text, &items);
                    let (inside, rest): (Vec<Splice>, Vec<Splice>) =
                        std::mem::take(&mut splices).into_iter().partition(|s| s.start >= start && s.end <= end);
                    splices = rest;
                    let mut inside = inside;
                    inside.sort_by_key(|s| s.start);
                    let piece = |s: usize, e: usize| {
                        let within: Vec<&Splice> = inside.iter().filter(|x| x.start >= s && x.end <= e).collect();
                        let mut o = String::new();
                        let mut at = s;
                        for x in within {
                            o.push_str(&text[at..x.start]);
                            o.push_str(&x.text);
                            at = x.end;
                        }
                        o.push_str(&text[at..e]);
                        o
                    };
                    let new_items = match call {
                        None => {
                            out.declarations += 1;
                            resolved.iter().map(|(np, _)| np.text.trim().to_string()).collect::<Vec<_>>()
                        }
                        Some(receiver) => {
                            // Arguments with the rewrites of calls nested in them.
                            let args: Vec<Arg> = arguments(text, list)
                                .into_iter()
                                .zip(&items)
                                .map(|(a, &(s, e))| {
                                    let full = piece(s, e);
                                    let value = match &a.name {
                                        Some(_) => full[full.find(':').map_or(0, |i| i + 1)..].trim().to_string(),
                                        None => full.clone(),
                                    };
                                    Arg { text: full, name: a.name, value }
                                })
                                .collect();
                            let is_receiver = sig.extension
                                && receiver.as_deref().is_some_and(|r| {
                                    sig.container.as_deref().is_none_or(|c| r != c && !r.ends_with(&format!(".{c}")))
                                });
                            match rewrite_arguments(sig, &resolved, &args, is_receiver) {
                                Ok(items) => {
                                    out.calls += 1;
                                    items
                                }
                                Err(reason) => {
                                    out.skipped.push(SkippedSite {
                                        path: file.path.clone(),
                                        range: lines.range(list.start_byte(), list.end_byte()),
                                        reason: reason.into(),
                                    });
                                    // Keep the nested rewrites.
                                    splices.push(Splice { start, end, text: spliced(text, start, end, &inside) });
                                    continue;
                                }
                            }
                        }
                    };
                    let multiline = items.len() > 1 && sep.contains('\n');
                    let joined = new_items.join(if multiline { &sep } else { ", " });
                    let leading = &text[start..items.first().map_or(start, |i| i.0)];
                    let trailing = &text[items.last().map_or(end, |i| i.1)..end];
                    let replacement = if multiline || leading.contains('\n') {
                        format!("{leading}{joined}{trailing}")
                    } else {
                        joined
                    };
                    splices.push(Splice { start, end, text: replacement });
                }
                splices.sort_by_key(|s| s.start);
                splices
                    .into_iter()
                    .filter(|s| text[s.start..s.end] != s.text)
                    .map(|s| TextEdit { range: lines.range(s.start, s.end), new_text: s.text })
                    .collect::<Vec<_>>()
            })
            .map_err(|e| format!("cannot parse {}: {e:?}", file.path.display()))?;
        if !edits.is_empty() {
            out.edit.files.push(FileEdit::Edit { path: file.path.clone(), edits, version: None });
        }
    }
    if !files.is_empty() && out.declarations == 0 {
        // Callers changed without their declaration would not compile.
        return Err(format!("The declaration of '{}' is not in the project", sig.name));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos_of(text: &str, needle: &str) -> Pos {
        Lines::new(text).pos(text.find(needle).unwrap())
    }

    fn apply(text: &str, rewrite: &SignatureRewrite) -> String {
        let Some(FileEdit::Edit { edits, .. }) = rewrite.edit.files.first() else { return text.to_string() };
        let lines = Lines::new(text);
        let mut out = text.to_string();
        let mut edits = edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(lines.byte(e.range.start)));
        for e in edits {
            out.replace_range(lines.byte(e.range.start)..lines.byte(e.range.end), &e.new_text);
        }
        out
    }

    /// Every occurrence of `name` as a whole word.
    fn locations(text: &str, name: &str) -> Vec<Range> {
        let lines = Lines::new(text);
        let bytes = text.as_bytes();
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        text.match_indices(name)
            .filter(|(i, _)| {
                let end = i + name.len();
                (*i == 0 || !word(bytes[i - 1])) && (end >= bytes.len() || !word(bytes[end]))
            })
            .map(|(i, _)| lines.range(i, i + name.len()))
            .collect()
    }

    fn run(
        text: &str,
        at: &str,
        name: &str,
        change: &[(Option<u32>, &str, Option<&str>)],
    ) -> (String, SignatureRewrite) {
        let syntax = Syntax::new();
        let sig = signature_at(&syntax, text, pos_of(text, at)).expect("a signature");
        let change: Vec<NewParameter> = change
            .iter()
            .map(|(from, t, v)| NewParameter { from: *from, text: t.to_string(), value: v.map(str::to_string) })
            .collect();
        let files = [SignatureSites { path: "/s/A.cs".into(), text: text.into(), locations: locations(text, name) }];
        let rewrite = change_signature(&syntax, &sig, &change, &files).unwrap();
        (apply(text, &rewrite), rewrite)
    }

    const PRICING: &str = r#"class Pricing {
    public decimal Total(decimal net, int count, string currency = "GBP") => net * count;
    void Use() {
        var a = Total(10m, 2);
        var b = Total(Total(1m, 1), 3, currency: "EUR");
        var c = this.Total(net: 5m, count: 1);
        System.Func<decimal, int, string, decimal> f = Total;
    }
}"#;

    #[test]
    fn reads_the_signature_under_the_caret() {
        let syntax = Syntax::new();
        let sig = signature_at(&syntax, PRICING, pos_of(PRICING, "Total(decimal")).unwrap();
        assert_eq!(sig.name, "Total");
        assert_eq!(sig.kind, "method");
        assert_eq!(sig.container.as_deref(), Some("Pricing"));
        let names: Vec<_> = sig.parameters.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["net", "count", "currency"]);
        assert!(sig.parameters[2].has_default);
        assert_eq!(sig.header, r#"public decimal Total(decimal net, int count, string currency = "GBP")"#);
        // From inside the body too.
        assert_eq!(signature_at(&syntax, PRICING, pos_of(PRICING, "net * count")).unwrap().name, "Total");
    }

    #[test]
    fn reorders_parameters_and_arguments() {
        let (out, rewrite) = run(
            PRICING,
            "Total(decimal",
            "Total",
            &[
                (Some(1), "int count", None),
                (Some(0), "decimal net", None),
                (Some(2), r#"string currency = "GBP""#, None),
            ],
        );
        assert!(out.contains(r#"Total(int count, decimal net, string currency = "GBP")"#), "{out}");
        assert!(out.contains("Total(2, 10m)"), "{out}");
        // The nested call is rewritten inside the outer one; named arguments stay named.
        assert!(out.contains(r#"Total(3, Total(1, 1m), currency: "EUR")"#), "{out}");
        assert!(out.contains("this.Total(count: 1, net: 5m)"), "{out}");
        assert_eq!((rewrite.declarations, rewrite.calls), (1, 4));
        assert_eq!(rewrite.skipped.len(), 1, "the method group");
        assert_eq!(rewrite.skipped[0].reason, "a method group, not a call");
    }

    #[test]
    fn removes_and_adds_parameters() {
        let (out, _) = run(
            PRICING,
            "Total(decimal",
            "Total",
            &[
                (Some(0), "decimal net", None),
                (Some(2), r#"string currency = "GBP""#, None),
                (None, "bool rounded", Some("true")),
            ],
        );
        assert!(out.contains(r#"Total(decimal net, string currency = "GBP", bool rounded)"#), "{out}");
        // The omitted optional argument makes what follows named.
        assert!(out.contains("Total(10m, rounded: true)"), "{out}");
        assert!(out.contains(r#"Total(Total(1m, rounded: true), currency: "EUR", rounded: true)"#), "{out}");
    }

    #[test]
    fn optional_new_parameters_are_left_to_their_default() {
        let (out, _) = run(
            PRICING,
            "Total(decimal",
            "Total",
            &[
                (Some(0), "decimal net", None),
                (Some(1), "int count", None),
                (None, "bool rounded = false", None),
                (Some(2), r#"string currency = "GBP""#, None),
            ],
        );
        assert!(out.contains("Total(10m, 2)"), "{out}");
        assert!(out.contains(r#"Total(Total(1m, 1), 3, currency: "EUR")"#), "{out}");
    }

    #[test]
    fn extension_methods_keep_their_receiver() {
        let text = r#"static class Ext {
    public static int Add(this int a, int b, params int[] rest) => a + b;
    static void Use() { var x = 5.Add(1, 2, 3); var y = Ext.Add(5, 1); var z = list?.Count.Add(4); }
}"#;
        let (out, rewrite) = run(
            text,
            "Add(this",
            "Add",
            &[
                (Some(0), "this int a", None),
                (None, "int scale", Some("10")),
                (Some(1), "int b", None),
                (Some(2), "params int[] rest", None),
            ],
        );
        assert!(out.contains("Add(this int a, int scale, int b, params int[] rest)"), "{out}");
        assert!(out.contains("5.Add(10, 1, 2, 3)"), "{out}");
        assert!(out.contains("Ext.Add(5, 10, 1)"), "{out}");
        assert!(out.contains("Count.Add(10, 4)"), "{out}");
        assert_eq!(rewrite.calls, 3);
    }

    #[test]
    fn constructors_and_their_callers() {
        let text = r#"class P {
    public P(int a, string s) { }
    public P(int a) : this(a, "x") { }
    static P Make() => new P(1, "y");
    static P Other() { P p = new(2, "z"); return p; }
}"#;
        let syntax = Syntax::new();
        let sig = signature_at(&syntax, text, pos_of(text, "P(int a, string")).unwrap();
        assert_eq!(sig.kind, "constructor");
        let change = [
            NewParameter { from: Some(1), text: "string s".into(), value: None },
            NewParameter { from: Some(0), text: "int a".into(), value: None },
        ];
        // Roslyn reports `this`, the type in `new P` and `new` in `new(...)`.
        let lines = Lines::new(text);
        let at = |needle: &str| {
            let i = text.find(needle).unwrap();
            lines.range(i, i + 1)
        };
        let locations = vec![sig.name_range, at("this(a"), at("P(1"), at("new(2")];
        let files = [SignatureSites { path: "/s/P.cs".into(), text: text.into(), locations }];
        let rewrite = change_signature(&syntax, &sig, &change, &files).unwrap();
        let out = apply(text, &rewrite);
        assert!(out.contains("public P(string s, int a) { }"), "{out}");
        assert!(out.contains(r#": this("x", a)"#), "{out}");
        assert!(out.contains(r#"new P("y", 1)"#), "{out}");
        assert!(out.contains(r#"new("z", 2)"#), "{out}");
        assert!(out.contains("public P(int a) :"), "the other constructor is untouched: {out}");
    }

    #[test]
    fn multi_line_lists_keep_their_layout() {
        let text = "class C {\n    void M(\n        int a,\n        int b)\n    { M(\n        1,\n        2); }\n}";
        let (out, _) = run(text, "M(\n", "M", &[(Some(1), "int b", None), (Some(0), "int a", None)]);
        assert!(out.contains("void M(\n        int b,\n        int a)"), "{out}");
        assert!(out.contains("M(\n        2,\n        1)"), "{out}");
    }

    #[test]
    fn parses_parameter_declarations() {
        let p = parameter("[NotNull] params Dictionary<string, int>[] maps");
        assert_eq!(
            (p.name.as_str(), p.modifiers.as_slice(), p.has_default),
            ("maps", &["params".to_string()][..], false)
        );
        let p = parameter(r#"string @class = "a, b""#);
        assert_eq!((p.name.as_str(), p.has_default), ("@class", true));
        let p = parameter("ref readonly int x");
        assert_eq!(p.modifiers, ["ref", "readonly"]);
    }

    #[test]
    fn splits_lists_with_generics_lambdas_and_comments() {
        let text = "class C { void M() { F(new List<int>(), G(x => x > 1), /* a, b */ 3, a < b, c > d); } }";
        let syntax = Syntax::new();
        let items = syntax
            .with_parsed(&csharp(), text, &|| false, |parsed| {
                let call = parsed
                    .tree
                    .root_node()
                    .descendant_for_byte_range(text.find("F(").unwrap(), text.find("F(").unwrap())
                    .unwrap();
                let list = call.parent().unwrap().child_by_field_name("arguments").unwrap();
                split_list(text, list).into_iter().map(|(s, e)| text[s..e].to_string()).collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(items, ["new List<int>()", "G(x => x > 1)", "/* a, b */ 3", "a < b", "c > d"]);
    }

    #[test]
    fn rejects_unsafe_changes() {
        let syntax = Syntax::new();
        let sig = signature_at(&syntax, PRICING, pos_of(PRICING, "Total(decimal")).unwrap();
        let try_change = |c: &[(Option<u32>, &str, Option<&str>)]| {
            let change: Vec<NewParameter> = c
                .iter()
                .map(|(from, t, v)| NewParameter { from: *from, text: t.to_string(), value: v.map(str::to_string) })
                .collect();
            change_signature(&syntax, &sig, &change, &[])
        };
        assert!(try_change(&[(Some(0), "decimal amount", None)]).unwrap_err().contains("Rename"));
        assert!(try_change(&[(None, "int extra", None)]).unwrap_err().contains("value"));
        assert!(
            try_change(&[(Some(0), "decimal net", None), (None, "int net", Some("1"))]).unwrap_err().contains("twice")
        );
        assert!(
            try_change(&[(None, "params int[] xs", None), (Some(0), "decimal net", None)])
                .unwrap_err()
                .contains("last")
        );
    }
}
