//! Trace frames → source: `dotnet-trace` names a managed frame
//! `Module!Namespace.Outer+Inner.Method(args)`, with no file or line. [`parse_frame`]
//! turns the name into the C# declaration it came from (undoing what the compiler
//! generates for async methods, iterators, lambdas, local functions, properties and
//! constructors), and [`find_in_outline`] finds that declaration in a file's outline.

use ide_core::{OutlineItem, Range, SymbolKind};

/// What kind of declaration a frame's method is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method,
    Constructor,
    Property,
    LocalFunction,
}

/// One declaration a frame may come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub kind: MemberKind,
}

/// The source declaration behind a managed trace frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRef {
    pub module: String,
    /// Dotted namespace, `""` for the global namespace.
    pub namespace: String,
    /// Declaring types, outermost first (generic arity and compiler-generated types removed).
    pub types: Vec<String>,
    /// Declarations to try, best first (a local function falls back to the method holding it).
    pub members: Vec<Member>,
    /// Parameter count, when the frame is the declared method itself (not a lambda or state machine).
    pub params: Option<usize>,
}

impl FrameRef {
    /// The innermost declaring type.
    pub fn type_name(&self) -> &str {
        self.types.last().map_or("", String::as_str)
    }
}

/// Removes `[...]` generic instantiations (balanced) and `` `N `` arity suffixes.
fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '`' if depth == 0 => {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
            }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Number of comma-separated items at the top level of `args`.
fn count_params(args: &str) -> usize {
    if args.trim().is_empty() {
        return 0;
    }
    let mut depth = 0i32;
    let mut n = 1;
    for c in args.chars() {
        match c {
            '[' | '<' | '(' => depth += 1,
            ']' | '>' | ')' => depth -= 1,
            ',' if depth == 0 => n += 1,
            _ => {}
        }
    }
    n
}

/// `<Name>suffix` → (`Name`, `suffix`).
fn generated(segment: &str) -> Option<(&str, &str)> {
    let rest = segment.strip_prefix('<')?;
    let close = rest.find('>')?;
    Some((&rest[..close], &rest[close + 1..]))
}

/// Parses a `dotnet-trace` frame name. `None` for native, runtime and
/// unrecognisable frames, and for top-level statements (`<Main>$`).
pub fn parse_frame(frame: &str) -> Option<FrameRef> {
    let (module, rest) = frame.split_once('!')?;
    if module.is_empty() || module == "?" || module == "UNMANAGED_CODE_TIME" {
        return None;
    }
    let (sig, args) = match rest.find('(') {
        Some(open) => (&rest[..open], Some(rest[open + 1..].strip_suffix(')').unwrap_or(&rest[open + 1..]))),
        None => (rest, None),
    };
    let sig = strip_generics(sig);
    let (qualified, member) = if let Some(q) = sig.strip_suffix("..ctor") {
        (q, ".ctor")
    } else if let Some(q) = sig.strip_suffix("..cctor") {
        (q, ".cctor")
    } else {
        sig.rsplit_once('.')?
    };
    let mut pieces = qualified.split('+');
    let first = pieces.next()?;
    let (namespace, outer) = first.rsplit_once('.').unwrap_or(("", first));
    let mut types = Vec::new();
    let mut member = member.to_owned();
    let mut params = args.map(count_params);
    for segment in std::iter::once(outer).chain(pieces) {
        match generated(segment) {
            // `<GetAsync>d__3.MoveNext`: an async method's or iterator's state machine.
            Some((name, suffix)) if suffix.starts_with("d__") && !name.is_empty() => {
                if member == "MoveNext" {
                    member = name.to_owned();
                    params = None;
                }
            }
            // `<>c`, `<>c__DisplayClass0_0`: lambda closures; the method says whose.
            Some(_) => {}
            None => types.push(segment.to_owned()),
        }
    }
    if types.is_empty() {
        return None;
    }
    let ctor =
        |types: &[String]| Member { name: types.last().cloned().unwrap_or_default(), kind: MemberKind::Constructor };
    let method = |name: &str| {
        if name == ".ctor" || name == ".cctor" {
            ctor(&types)
        } else {
            Member { name: name.to_owned(), kind: MemberKind::Method }
        }
    };
    let members = if member == ".ctor" || member == ".cctor" {
        vec![ctor(&types)]
    } else if let Some((outer, suffix)) = generated(&member) {
        params = None;
        if outer.is_empty() || suffix == "$" {
            return None;
        }
        if let Some(local) = suffix.strip_prefix("g__") {
            // `<Main>g__Local|0_1`: a local function.
            let local = local.split('|').next().unwrap_or(local);
            vec![Member { name: local.to_owned(), kind: MemberKind::LocalFunction }, method(outer)]
        } else {
            // `<Main>b__0_0` (lambda), `<Main>` (synthesised async entry point).
            vec![method(outer)]
        }
    } else if let Some(prop) =
        ["get_", "set_", "init_"].iter().find_map(|p| member.strip_prefix(p)).filter(|p| !p.is_empty())
    {
        params = None;
        vec![Member { name: prop.to_owned(), kind: MemberKind::Property }, method(&member)]
    } else {
        vec![method(&member)]
    };
    Some(FrameRef { module: module.to_owned(), namespace: namespace.to_owned(), types, members, params })
}

/// Where a declaration is: the name (where navigation lands) and the whole extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub selection: Range,
    pub range: Range,
}

fn kind_matches(member: MemberKind, kind: SymbolKind) -> bool {
    match member {
        MemberKind::Method => matches!(kind, SymbolKind::Method | SymbolKind::Operator),
        MemberKind::Constructor => kind == SymbolKind::Constructor,
        MemberKind::Property => matches!(kind, SymbolKind::Property | SymbolKind::Event),
        MemberKind::LocalFunction => kind == SymbolKind::Function,
    }
}

fn is_type(kind: SymbolKind) -> bool {
    kind.is_type_like()
}

/// Parameter count of an outline `detail` such as `(int a, List<string> b)`.
fn detail_params(detail: Option<&str>) -> Option<usize> {
    let d = detail?.trim();
    let inner = d.strip_prefix('(')?.strip_suffix(')')?;
    Some(count_params(inner))
}

struct Candidate {
    found: Found,
    params: Option<usize>,
}

struct Search<'a> {
    frame: &'a FrameRef,
    member: &'a Member,
    /// The file's namespace when it is file-scoped (`namespace X;`).
    file_namespace: &'a str,
    out: Vec<Candidate>,
}

impl Search<'_> {
    fn walk(&mut self, items: &[OutlineItem], namespace: &str, types: &mut Vec<String>, in_method: Option<&str>) {
        for item in items {
            match item.kind {
                SymbolKind::Namespace => {
                    let ns =
                        if namespace.is_empty() { item.name.clone() } else { format!("{namespace}.{}", item.name) };
                    self.walk(&item.children, &ns, types, None);
                }
                k if is_type(k) => {
                    types.push(item.name.clone());
                    if self.types_match(namespace, types) && self.member.kind == MemberKind::Constructor {
                        // A primary constructor (`class C(int x)`) has no declaration of its own.
                        self.push_type(item);
                    }
                    self.walk(&item.children, namespace, types, None);
                    types.pop();
                }
                k => {
                    let here = self.types_match(namespace, types)
                        && kind_matches(self.member.kind, k)
                        && item.name == self.member.name
                        && (self.member.kind != MemberKind::LocalFunction || in_method.is_some());
                    if here {
                        self.out.push(Candidate {
                            found: Found { selection: item.selection_range, range: item.range },
                            params: detail_params(item.detail.as_deref()),
                        });
                    }
                    let holder = matches!(k, SymbolKind::Method | SymbolKind::Constructor | SymbolKind::Function);
                    self.walk(&item.children, namespace, types, holder.then_some(item.name.as_str()).or(in_method));
                }
            }
        }
    }

    fn types_match(&self, namespace: &str, types: &[String]) -> bool {
        let ns = if namespace.is_empty() { self.file_namespace } else { namespace };
        ns == self.frame.namespace && types == self.frame.types.as_slice()
    }

    fn push_type(&mut self, item: &OutlineItem) {
        // Ranked after real constructors by `params: None` and position.
        self.out.push(Candidate { found: Found { selection: item.selection_range, range: item.range }, params: None });
    }
}

/// Finds the declaration behind `frame` in a C# file's outline: the first of its
/// [`FrameRef::members`] that is declared there, preferring the overload with the
/// frame's parameter count.
pub fn find_in_outline(outline: &[OutlineItem], frame: &FrameRef) -> Option<Found> {
    // `namespace X;` appears as a leaf namespace item before the file's types.
    let file_namespace = outline
        .iter()
        .find(|i| i.kind == SymbolKind::Namespace && i.children.is_empty())
        .map_or("", |i| i.name.as_str());
    for member in &frame.members {
        let mut search = Search { frame, member, file_namespace, out: Vec::new() };
        search.walk(outline, "", &mut Vec::new(), None);
        let mut out = search.out;
        if member.kind == MemberKind::Constructor {
            // Declared constructors before the primary-constructor type fallback.
            out.sort_by_key(|c| c.params.is_none());
        }
        let best = match frame.params {
            Some(n) => out.iter().position(|c| c.params == Some(n)).unwrap_or(0),
            None => 0,
        };
        if let Some(c) = out.get(best) {
            return Some(c.found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::Pos;

    fn m(name: &str, kind: MemberKind) -> Member {
        Member { name: name.into(), kind }
    }

    #[test]
    fn parses_frame_names_from_dotnet_trace() {
        let f = parse_frame("Hot!Hot.Work.Calculator.Spin()").unwrap();
        assert_eq!((f.module.as_str(), f.namespace.as_str()), ("Hot", "Hot.Work"));
        assert_eq!(f.types, ["Calculator"]);
        assert_eq!(f.members, [m("Spin", MemberKind::Method)]);
        assert_eq!(f.params, Some(0));

        let f = parse_frame("Hot!Hot.Work.Worker+<RunAsync>d__0.MoveNext()").unwrap();
        assert_eq!(
            (f.types.as_slice(), f.members.as_slice(), f.params),
            (&["Worker".to_owned()][..], &[m("RunAsync", MemberKind::Method)][..], None)
        );

        let f = parse_frame("Hot!Hot.Work.Generic`1[System.__Canon].Build(!0)").unwrap();
        assert_eq!((f.types.as_slice(), f.params), (&["Generic".to_owned()][..], Some(1)));

        let f = parse_frame("Hot!Hot.Work.Calculator+Nested..ctor()").unwrap();
        assert_eq!(f.types, ["Calculator", "Nested"]);
        assert_eq!(f.members, [m("Nested", MemberKind::Constructor)]);

        let f = parse_frame("Hot!Hot.Work.Calculator.get_Total()").unwrap();
        assert_eq!(f.members, [m("Total", MemberKind::Property), m("get_Total", MemberKind::Method)]);

        let f = parse_frame("Hot!Hot.Work.Program+<>c.<Main>b__0_0(int32)").unwrap();
        assert_eq!(
            (f.types.as_slice(), f.members.as_slice()),
            (&["Program".to_owned()][..], &[m("Main", MemberKind::Method)][..])
        );

        let f = parse_frame("Hot!Hot.Work.Program.<Main>g__Local|0_1(int32)").unwrap();
        assert_eq!(f.members, [m("Local", MemberKind::LocalFunction), m("Main", MemberKind::Method)]);

        let f = parse_frame("Hot!Hot.Work.Program.<Main>()").unwrap();
        assert_eq!(f.members, [m("Main", MemberKind::Method)]);

        let f = parse_frame("App!Program.Run(class System.String[],value class System.Span`1<int32>)").unwrap();
        assert_eq!((f.namespace.as_str(), f.params), ("", Some(2)));

        let f = parse_frame("App!N.Box`1+<Items>d__4[System.Int32].MoveNext()").unwrap();
        assert_eq!(
            (f.types.as_slice(), f.members.as_slice()),
            (&["Box".to_owned()][..], &[m("Items", MemberKind::Method)][..])
        );

        assert!(parse_frame("Fixture.Api!Program.<Main>$(class System.String[])").is_none());
        assert!(parse_frame("UNMANAGED_CODE_TIME").is_none());
        assert!(parse_frame("?!?").is_none());
        assert!(parse_frame("Process64 Process(1234) (1234) Args: ").is_none());
    }

    fn item(
        name: &str,
        kind: SymbolKind,
        line: u32,
        end: u32,
        detail: Option<&str>,
        children: Vec<OutlineItem>,
    ) -> OutlineItem {
        let r = |a: u32, b: u32| Range { start: Pos { line: a, col: 0 }, end: Pos { line: b, col: 0 } };
        OutlineItem {
            name: name.into(),
            detail: detail.map(Into::into),
            kind,
            range: r(line, end),
            selection_range: r(line, line),
            children,
            visibility: None,
        }
    }

    #[test]
    fn finds_declarations_in_an_outline() {
        use SymbolKind as K;
        let outline = vec![
            item("Hot.Work", K::Namespace, 0, 0, None, vec![]),
            item(
                "Calculator",
                K::Class,
                2,
                30,
                None,
                vec![
                    item("Spin", K::Method, 4, 6, Some("()"), vec![]),
                    item("Spin", K::Method, 7, 9, Some("(int times)"), vec![]),
                    item("Total", K::Property, 10, 12, None, vec![]),
                    item(
                        "Run",
                        K::Method,
                        13,
                        20,
                        Some("()"),
                        vec![item("Local", K::Function, 15, 16, Some("(int x)"), vec![])],
                    ),
                    item(
                        "Nested",
                        K::Class,
                        21,
                        29,
                        None,
                        vec![item("Nested", K::Constructor, 22, 24, Some("()"), vec![])],
                    ),
                ],
            ),
            item("Primary", K::Class, 31, 33, None, vec![]),
        ];
        let line = |frame: &str| {
            find_in_outline(&outline, &parse_frame(frame).unwrap()).map(|f| (f.selection.start.line, f.range.end.line))
        };
        assert_eq!(line("Hot!Hot.Work.Calculator.Spin()"), Some((4, 6)));
        assert_eq!(line("Hot!Hot.Work.Calculator.Spin(int32)"), Some((7, 9)));
        assert_eq!(line("Hot!Hot.Work.Calculator.get_Total()"), Some((10, 12)));
        assert_eq!(line("Hot!Hot.Work.Calculator.<Run>g__Local|3_0(int32)"), Some((15, 16)));
        assert_eq!(line("Hot!Hot.Work.Calculator+<>c.<Run>b__3_1()"), Some((13, 20)));
        assert_eq!(line("Hot!Hot.Work.Calculator+Nested..ctor()"), Some((22, 24)));
        assert_eq!(line("Hot!Hot.Work.Primary..ctor(int32)"), Some((31, 33)));
        // Another namespace or another class.
        assert_eq!(line("Hot!Other.Calculator.Spin()"), None);
        assert_eq!(line("Hot!Hot.Work.Nested..ctor()"), None);
    }
}
