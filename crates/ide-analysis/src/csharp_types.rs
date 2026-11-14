//! The types a C# file declares and the members a XAML binding can name
//! (CI-16): public properties, `[ObservableProperty]` fields and partial
//! properties (CommunityToolkit.Mvvm), and `[RelayCommand]` methods as the
//! `…Command` properties the toolkit generates.

use crate::lines::Lines;
use ide_core::{LanguageId, Pos, Range};
use ide_language::Syntax;
use serde::{Deserialize, Serialize};
use specta::Type;
use tree_sitter::Node;

const TYPES: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "record_declaration",
    "record_struct_declaration",
    "interface_declaration",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MemberKind {
    Property,
    /// A `[RelayCommand]` method's generated command property.
    Command,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TypeMember {
    pub name: String,
    /// As written, e.g. `ObservableCollection<Item>?`; `None` for commands.
    pub type_name: Option<String>,
    pub kind: MemberKind,
    /// Where it is declared (the field or method for generated members).
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredType {
    pub name: String,
    /// Dotted; empty in the global namespace.
    pub namespace: String,
    /// Base type and interfaces as written, generic arguments dropped.
    pub bases: Vec<String>,
    pub members: Vec<TypeMember>,
    pub name_range: Range,
}

/// The types `text` declares (nested ones by their own name).
pub fn declared_types(syntax: &Syntax, text: &str) -> Vec<DeclaredType> {
    let lines = Lines::new(text);
    syntax
        .with_parsed(&LanguageId::new("csharp"), text, &|| false, |parsed| {
            let root = parsed.tree.root_node();
            let file_namespace = children(root)
                .find(|c| c.kind() == "file_scoped_namespace_declaration")
                .and_then(|n| n.child_by_field_name("name"))
                .map(|n| text[n.byte_range()].to_string());
            let mut out = Vec::new();
            collect(root, text, &lines, file_namespace.as_deref().unwrap_or(""), &mut out);
            out
        })
        .unwrap_or_default()
}

fn children<'t>(node: Node<'t>) -> impl Iterator<Item = Node<'t>> {
    (0..node.named_child_count()).filter_map(move |i| node.named_child(i))
}

fn collect(node: Node, text: &str, lines: &Lines, namespace: &str, out: &mut Vec<DeclaredType>) {
    for child in children(node) {
        match child.kind() {
            "namespace_declaration" => {
                let name = child.child_by_field_name("name").map_or("", |n| &text[n.byte_range()]);
                let inner = if namespace.is_empty() { name.to_string() } else { format!("{namespace}.{name}") };
                if let Some(body) = child.child_by_field_name("body") {
                    collect(body, text, lines, &inner, out);
                }
            }
            "declaration_list" => collect(child, text, lines, namespace, out),
            k if TYPES.contains(&k) => {
                if let Some(t) = declared(child, text, lines, namespace) {
                    out.push(t);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    collect(body, text, lines, namespace, out);
                }
            }
            _ => {}
        }
    }
}

fn declared(node: Node, text: &str, lines: &Lines, namespace: &str) -> Option<DeclaredType> {
    let name = node.child_by_field_name("name")?;
    let bases = children(node)
        .find(|c| c.kind() == "base_list")
        .map(|list| {
            children(list)
                .filter(|c| c.kind() != "argument_list")
                .map(|c| {
                    let t = &text[c.byte_range()];
                    t.split('<').next().unwrap_or(t).trim().to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let interface = node.kind() == "interface_declaration";
    let mut members = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        for m in children(body) {
            let attrs = attributes(m, text);
            let has = |a: &str| attrs.iter().any(|x| x == a);
            match m.kind() {
                "property_declaration" if interface || is_public(m, text) || has("ObservableProperty") => {
                    if let Some(n) = m.child_by_field_name("name") {
                        members.push(TypeMember {
                            name: text[n.byte_range()].to_string(),
                            type_name: m.child_by_field_name("type").map(|t| text[t.byte_range()].to_string()),
                            kind: MemberKind::Property,
                            range: lines.range(n.start_byte(), n.end_byte()),
                        });
                    }
                }
                "field_declaration" if has("ObservableProperty") => {
                    let Some(decl) = children(m).find(|c| c.kind() == "variable_declaration") else { continue };
                    let ty = decl.child_by_field_name("type").map(|t| text[t.byte_range()].to_string());
                    for v in children(decl).filter(|c| c.kind() == "variable_declarator") {
                        let Some(n) = v.child_by_field_name("name") else { continue };
                        members.push(TypeMember {
                            name: generated_property(&text[n.byte_range()]),
                            type_name: ty.clone(),
                            kind: MemberKind::Property,
                            range: lines.range(n.start_byte(), n.end_byte()),
                        });
                    }
                }
                "method_declaration" if has("RelayCommand") => {
                    if let Some(n) = m.child_by_field_name("name") {
                        let method = &text[n.byte_range()];
                        members.push(TypeMember {
                            name: format!("{}Command", method.strip_suffix("Async").unwrap_or(method)),
                            type_name: None,
                            kind: MemberKind::Command,
                            range: lines.range(n.start_byte(), n.end_byte()),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    Some(DeclaredType {
        name: text[name.byte_range()].to_string(),
        namespace: namespace.to_string(),
        bases,
        members,
        name_range: lines.range(name.start_byte(), name.end_byte()),
    })
}

/// Attribute names on a member, without an `Attribute` suffix or namespace.
fn attributes(node: Node, text: &str) -> Vec<String> {
    children(node)
        .filter(|c| c.kind() == "attribute_list")
        .flat_map(children)
        .filter(|a| a.kind() == "attribute")
        .filter_map(|a| a.child_by_field_name("name"))
        .map(|n| {
            let full = &text[n.byte_range()];
            let last = full.rsplit('.').next().unwrap_or(full);
            last.strip_suffix("Attribute").unwrap_or(last).to_string()
        })
        .collect()
}

fn is_public(node: Node, text: &str) -> bool {
    children(node).any(|c| c.kind() == "modifier" && &text[c.byte_range()] == "public")
}

/// The property CommunityToolkit.Mvvm generates for an `[ObservableProperty]` field:
/// `_name`, `m_name` and `name` all become `Name`.
pub fn generated_property(field: &str) -> String {
    let bare = field.strip_prefix("m_").unwrap_or(field).trim_start_matches('_');
    let mut chars = bare.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => field.to_string(),
    }
}

/// The element type a binding path continues into: `User?` is `User`,
/// `ObservableCollection<Item>` and `Item[]` are `Item` (after an indexer), others as written.
pub fn simple_type_name(type_name: &str) -> String {
    let t = type_name.trim().trim_end_matches('?').trim();
    if let Some(element) = t.strip_suffix("[]") {
        return simple_type_name(element);
    }
    let base = t.split('<').next().unwrap_or(t);
    base.rsplit('.').next().unwrap_or(base).trim().to_string()
}

/// Every declaration that makes a type of its own in the assembly.
const METADATA_TYPES: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "record_declaration",
    "record_struct_declaration",
    "interface_declaration",
    "enum_declaration",
    "delegate_declaration",
];

/// The metadata name of the type declared around `pos`, as reflection and IL spell it:
/// `Namespace.Outer`1+Inner`. Without a position, or outside every type, the first type
/// the file declares; `Program` for a file of top-level statements only.
pub fn metadata_name_at(syntax: &Syntax, text: &str, pos: Option<Pos>) -> Option<String> {
    let lines = Lines::new(text);
    syntax
        .with_parsed(&LanguageId::new("csharp"), text, &|| false, |parsed| {
            let root = parsed.tree.root_node();
            let around = pos.and_then(|p| {
                let byte = lines.byte(p);
                let mut node = root.descendant_for_byte_range(byte, byte)?;
                while !METADATA_TYPES.contains(&node.kind()) {
                    node = node.parent()?;
                }
                Some(node)
            });
            match around.or_else(|| first_type(root)) {
                Some(node) => metadata_name(node, text),
                None => children(root).any(|c| c.kind() == "global_statement").then(|| "Program".to_string()),
            }
        })
        .ok()
        .flatten()
}

fn first_type(node: Node) -> Option<Node> {
    children(node).find_map(|child| match child.kind() {
        k if METADATA_TYPES.contains(&k) => Some(child),
        "namespace_declaration" | "file_scoped_namespace_declaration" | "declaration_list" => first_type(child),
        _ => None,
    })
}

fn metadata_name(node: Node, text: &str) -> Option<String> {
    let mut types = Vec::new();
    let mut namespaces = Vec::new();
    let mut at = Some(node);
    while let Some(n) = at {
        let name = || n.child_by_field_name("name").map(|name| &text[name.byte_range()]);
        match n.kind() {
            k if METADATA_TYPES.contains(&k) => {
                let arity = children(n)
                    .find(|c| c.kind() == "type_parameter_list")
                    .map_or(0, |list| children(list).filter(|c| c.kind() == "type_parameter").count());
                types.push(if arity == 0 { name()?.to_string() } else { format!("{}`{arity}", name()?) });
            }
            "namespace_declaration" | "file_scoped_namespace_declaration" => namespaces.push(name()?),
            // `namespace A.B;` is followed by its types rather than containing them.
            "compilation_unit" => namespaces.extend(
                children(n)
                    .find(|c| c.kind() == "file_scoped_namespace_declaration" && c.end_byte() <= node.start_byte())
                    .and_then(|c| c.child_by_field_name("name"))
                    .map(|name| &text[name.byte_range()]),
            ),
            _ => {}
        }
        at = n.parent();
    }
    types.reverse();
    namespaces.reverse();
    let namespace = namespaces.join(".").split_whitespace().collect::<String>();
    let types = types.join("+");
    Some(if namespace.is_empty() { types } else { format!("{namespace}.{types}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM: &str = r#"using CommunityToolkit.Mvvm.ComponentModel;
namespace App.ViewModels;

public partial class MainViewModel : ViewModelBase, INotifyPropertyChanged
{
    [ObservableProperty] private string _name = "";
    [ObservableProperty] public partial int Count { get; set; }
    [property: JsonIgnore] [ObservableProperty] private bool m_busy;
    public ObservableCollection<Item>? Items { get; } = new();
    [RelayCommand] private async Task SaveAsync() { }
    [CommunityToolkit.Mvvm.Input.RelayCommandAttribute] private void Reset() { }
    public User Owner => null!;
    private int Hidden { get; set; }
    public int field;

    public class Nested { public string Label { get; set; } }
}
"#;

    #[test]
    fn finds_the_bindable_members_of_a_view_model() {
        let types = declared_types(&Syntax::new(), VM);
        let vm = types.iter().find(|t| t.name == "MainViewModel").unwrap();
        assert_eq!(vm.namespace, "App.ViewModels");
        assert_eq!(vm.bases, ["ViewModelBase", "INotifyPropertyChanged"]);
        let names: Vec<(&str, Option<&str>)> =
            vm.members.iter().map(|m| (m.name.as_str(), m.type_name.as_deref())).collect();
        assert_eq!(
            names,
            [
                ("Name", Some("string")),
                ("Count", Some("int")),
                ("Busy", Some("bool")),
                ("Items", Some("ObservableCollection<Item>?")),
                ("SaveCommand", None),
                ("ResetCommand", None),
                ("Owner", Some("User")),
            ]
        );
        let nested = types.iter().find(|t| t.name == "Nested").unwrap();
        assert_eq!(nested.members[0].name, "Label");
    }

    #[test]
    fn namespaces_nest_and_interfaces_list_every_property() {
        let text = "namespace A { namespace B { interface IShape { double Area { get; } } } }";
        let types = declared_types(&Syntax::new(), text);
        assert_eq!(types[0].namespace, "A.B");
        assert_eq!(types[0].members[0].name, "Area");
    }

    #[test]
    fn names_of_generated_properties_and_path_types() {
        assert_eq!(generated_property("_firstName"), "FirstName");
        assert_eq!(generated_property("m_count"), "Count");
        assert_eq!(generated_property("title"), "Title");
        assert_eq!(simple_type_name("ObservableCollection<Item>?"), "ObservableCollection");
        assert_eq!(simple_type_name("Models.User?"), "User");
        assert_eq!(simple_type_name("Item[]"), "Item");
    }

    const NAMES: &str = "namespace A.B;\npublic class Outer<T>\n{\n    public struct Inner<U, V> { void M() { } }\n    public delegate R Make<R>();\n    enum Kind { One }\n}\nrecord Point(int X);\n";

    #[test]
    fn metadata_names_count_type_parameters_and_nest_with_a_plus() {
        let syntax = Syntax::new();
        let at = |needle: &str| {
            let byte = NAMES.find(needle).unwrap();
            let line = NAMES[..byte].matches('\n').count();
            let col = byte - NAMES[..byte].rfind('\n').map_or(0, |i| i + 1);
            metadata_name_at(&syntax, NAMES, Some(Pos::new(line as u32, col as u32)))
        };
        assert_eq!(at("M()").as_deref(), Some("A.B.Outer`1+Inner`2"));
        assert_eq!(at("Make").as_deref(), Some("A.B.Outer`1+Make`1"));
        assert_eq!(at("One").as_deref(), Some("A.B.Outer`1+Kind"));
        assert_eq!(at("public class").as_deref(), Some("A.B.Outer`1"));
        assert_eq!(at("Point").as_deref(), Some("A.B.Point"));
        // Outside every type, and with no position: the first type of the file.
        assert_eq!(at("namespace").as_deref(), Some("A.B.Outer`1"));
        assert_eq!(metadata_name_at(&syntax, NAMES, None).as_deref(), Some("A.B.Outer`1"));
    }

    #[test]
    fn metadata_names_follow_block_namespaces_and_top_level_statements() {
        let syntax = Syntax::new();
        let nested = "namespace A { namespace B.C { interface IShape<in T> { } } }";
        assert_eq!(metadata_name_at(&syntax, nested, None).as_deref(), Some("A.B.C.IShape`1"));
        assert_eq!(metadata_name_at(&syntax, "class Plain { }", None).as_deref(), Some("Plain"));
        assert_eq!(metadata_name_at(&syntax, "System.Console.WriteLine(1);\n", None).as_deref(), Some("Program"));
        assert_eq!(metadata_name_at(&syntax, "using System;\n", None), None);
    }
}
