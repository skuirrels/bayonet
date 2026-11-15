//! Client-side help for netcoredbg's expression evaluator.
//!
//! netcoredbg evaluates member paths, indexers, operators and method calls, but no
//! lambdas, no unqualified type names (`Math.Max`, `string.IsNullOrEmpty`), no `x == null`
//! on references and no `Length`. When its own evaluation fails, [`assist`] rewrites the
//! expression into pieces it can evaluate:
//!
//! - a LINQ method chain over an array, `List<T>`, `Dictionary<K, V>` or `HashSet<T>` is
//!   run here: the elements are read through `variables`, and each lambda body is
//!   evaluated by the adapter with the parameter replaced by the element's own
//!   expression (`list._items[0] > 2`);
//! - `&&` / `||` short-circuit here, and `x == null` reads `x`;
//! - `text.Length` becomes `text.get_Length()`, an array's `Length` its element count;
//! - a type name is qualified with the namespaces the frame's source imports.
//!
//! Values are only ever computed by the adapter or by exact integer arithmetic here;
//! whatever is not understood is an error, never a guess.

use crate::Variable;
use crate::connection::DapError;
use crate::session::Session;
use futures::future::BoxFuture;
use ide_core::{IdeError, IdeResult};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::{Duration, Instant};

/// The adapter this module helps.
pub(crate) const ADAPTER: &str = "netcoredbg";

/// Sequence results live in the session, not in the adapter: their references start here.
const SYNTHETIC_BASE: i64 = 1 << 40;
/// Largest collection a LINQ chain is run over.
pub(crate) const MAX_ELEMENTS: usize = 10_000;
/// One evaluation, with all its adapter round trips.
const TIME_BUDGET: Duration = Duration::from_secs(10);
/// Helpers evaluate rewritten expressions, which may need help themselves.
const MAX_DEPTH: usize = 8;
/// netcoredbg's parser runs out of stack on long operator chains (200 terms work, 300 do not).
const MAX_TERMS: usize = 100;
/// Combinations of namespaces tried for the type names of one expression.
const MAX_QUALIFICATIONS: usize = 32;

const SUPPORTED: &str = "Where, Select, SelectMany, Count, Any, All, First, Last, Single, ElementAt (and their \
                         OrDefault forms), Take, Skip, TakeWhile, SkipWhile, Sum, Min, Max, Average, Contains, \
                         Distinct, OrderBy, OrderByDescending, ToList, ToArray";

/// Children of the sequence results of one session, dropped when the program resumes.
#[derive(Default)]
pub(crate) struct Synthetic {
    next: i64,
    nodes: HashMap<i64, Vec<Variable>>,
    epoch: u64,
}

impl Synthetic {
    pub fn is(reference: i64) -> bool {
        reference >= SYNTHETIC_BASE
    }

    pub fn insert(&mut self, children: Vec<Variable>) -> i64 {
        let id = SYNTHETIC_BASE + self.next;
        self.next += 1;
        self.nodes.insert(id, children);
        id
    }

    pub fn get(&self, reference: i64) -> Option<Vec<Variable>> {
        self.nodes.get(&reference).cloned()
    }

    /// The program resumed: every reference is stale, and running evaluations stop.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.epoch += 1;
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// What the evaluator needs from the debug session.
pub(crate) trait Host: Send + Sync {
    fn evaluate<'a>(&'a self, expression: &'a str) -> BoxFuture<'a, Result<Variable, DapError>>;
    /// The first `count` children (all when `None`).
    fn variables(&self, reference: i64, count: Option<usize>) -> BoxFuture<'_, Result<Vec<Variable>, DapError>>;
    /// Stores the children of a sequence result and returns their reference.
    fn keep(&self, children: Vec<Variable>) -> i64;
    /// The program resumed or the session ended.
    fn cancelled(&self) -> bool;
    /// Namespaces an unqualified type name of the frame may live in, nearest first.
    fn namespaces(&self) -> BoxFuture<'_, Vec<String>>;
}

enum Fail {
    /// The helper does not apply (with the reason, when there is one worth showing).
    Skip(String),
    Error(String),
    /// Transport failure, timeout or cancellation.
    Fatal(IdeError),
}

type Outcome<T> = Result<T, Fail>;

fn error<T>(message: impl Into<String>) -> Outcome<T> {
    Err(Fail::Error(message.into()))
}

/// Evaluates `expression` after the adapter answered `native` for it.
pub(crate) async fn assist(
    host: &dyn Host,
    expression: &str,
    native: Result<Variable, DapError>,
) -> IdeResult<Variable> {
    let native = match native {
        Ok(v) if !suspicious(&v) => return Ok(v),
        Ok(v) => Ok(v),
        Err(DapError::Failed { command, message }) => Err((command, message)),
        Err(e) => return Err(e.into()),
    };
    let mut evaluator = Evaluator::new(host);
    let answer = native.clone().map_err(|(_, message)| message);
    match evaluator.help(expression, &answer).await {
        Ok(mut v) => {
            v.name = expression.to_owned();
            v.evaluate_name = Some(expression.to_owned());
            Ok(v)
        }
        Err(Fail::Fatal(e)) => Err(e),
        Err(other) => match (native, other) {
            (Ok(v), _) => Ok(v),
            (Err(_), Fail::Error(message)) => Err(IdeError::invalid(message)),
            (Err(_), Fail::Skip(reason)) if !reason.is_empty() && expression.contains("=>") => {
                Err(IdeError::invalid(reason))
            }
            (Err(_), Fail::Skip(_)) if expression.contains("=>") => Err(IdeError::invalid(format!(
                "netcoredbg cannot evaluate lambdas, and this is not a LINQ call that can be run for it ({SUPPORTED})"
            ))),
            (Err((command, message)), _) => Err(DapError::Failed { command, message }.into()),
        },
    }
}

/// An answer that is not the value asked for: a lazy LINQ iterator (its fields, not its
/// elements), or the reflection error netcoredbg returns for `Sum()`, `Max()`, `dict.First()`.
fn suspicious(v: &Variable) -> bool {
    let ty = v.type_name.as_deref().unwrap_or_default();
    ty.starts_with("System.Linq.") || ty == "System.Reflection.TargetParameterCountException"
}

// --- Tokens ----------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Ident,
    Number,
    /// A string or character literal.
    Text,
    Punct,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Token {
    fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    fn is(&self, src: &str, punct: &str) -> bool {
        self.kind == Kind::Punct && self.text(src) == punct
    }

    fn word(&self, src: &str, word: &str) -> bool {
        self.kind == Kind::Ident && self.text(src) == word
    }
}

const PUNCT: &[&str] = &[
    "<<=", "??=", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=",
    "|=", "^=", "<<", "::", "->",
];

const ASSIGNMENTS: &[&str] = &["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", "??=", "++", "--"];

/// Splits a C# expression into tokens. Interpolated strings and comments are refused:
/// their contents cannot be told from code without a full parser.
fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(c) = src[i..].chars().next() {
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        let rest = &src[i..];
        let kind = if rest.starts_with("//") || rest.starts_with("/*") {
            return Err("comments are not supported in expressions".into());
        } else if c == '$' || (c == '@' && rest[1..].starts_with('$')) {
            return Err("interpolated strings are not supported".into());
        } else if c == '"' || (c == '@' && rest[1..].starts_with('"')) {
            i = string_end(src, i).ok_or("unterminated string literal")?;
            Kind::Text
        } else if c == '\'' {
            i = char_end(src, i).ok_or("unterminated character literal")?;
            Kind::Text
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            i += c.len_utf8();
            i += src[i..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(src.len() - i);
            Kind::Ident
        } else if c.is_ascii_digit() {
            i = number_end(src, i);
            Kind::Number
        } else {
            let op = PUNCT.iter().find(|p| rest.starts_with(**p)).copied();
            // `a ? .5 : b` is a conditional, not `?.`.
            let op = op.filter(|p| *p != "?." || !rest[2..].starts_with(|c: char| c.is_ascii_digit()));
            i += op.map_or(c.len_utf8(), str::len);
            Kind::Punct
        };
        tokens.push(Token { kind, start, end: i });
    }
    Ok(tokens)
}

fn string_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let verbatim = bytes[start] == b'@';
    let mut i = start + usize::from(verbatim);
    let quotes = bytes[i..].iter().take_while(|b| **b == b'"').count();
    if quotes >= 3 {
        // A raw string ends at the same number of quotes.
        let close = "\"".repeat(quotes);
        return src[i + quotes..].find(&close).map(|at| i + quotes + at + quotes);
    }
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' if verbatim && bytes.get(i + 1) == Some(&b'"') => i += 2,
            b'"' => return Some(i + 1),
            b'\\' if !verbatim => i += 2,
            _ => i += 1,
        }
    }
    None
}

fn char_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

fn number_end(src: &str, start: usize) -> usize {
    let bytes = src.as_bytes();
    let hex = src[start..].starts_with("0x") || src[start..].starts_with("0X");
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        let digit_follows = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
        let exponent = !hex && i > start && matches!(bytes[i - 1], b'e' | b'E');
        let sign = matches!(b, b'+' | b'-') && exponent && digit_follows;
        if !(b.is_ascii_alphanumeric() || b == b'_' || (b == b'.' && digit_follows) || sign) {
            break;
        }
        i += 1;
    }
    i
}

fn depth_change(src: &str, t: &Token) -> i32 {
    if t.kind != Kind::Punct {
        return 0;
    }
    match t.text(src) {
        "(" | "[" | "{" => 1,
        ")" | "]" | "}" => -1,
        _ => 0,
    }
}

/// Index of the bracket closing the one at `open`.
fn closing(src: &str, tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        depth += depth_change(src, t);
        if depth == 0 {
            return Some(i);
        }
    }
    None
}

/// Index of the bracket opening the one at `close`.
fn opening(src: &str, tokens: &[Token], close: usize) -> Option<usize> {
    let mut depth = 0;
    for i in (0..=close).rev() {
        depth += depth_change(src, &tokens[i]);
        if depth == 0 {
            return Some(i);
        }
    }
    None
}

/// Indices in `lo..hi` of the tokens outside any bracket (the outermost brackets included).
fn top_level(src: &str, tokens: &[Token], lo: usize, hi: usize) -> Vec<usize> {
    let mut depth = 0;
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate().take(hi).skip(lo) {
        let change = depth_change(src, t);
        depth += change.min(0);
        if depth == 0 {
            out.push(i);
        }
        depth += change.max(0);
    }
    out
}

/// Source text of the token range `lo..hi`.
fn slice<'a>(src: &'a str, tokens: &[Token], lo: usize, hi: usize) -> &'a str {
    if lo >= hi { "" } else { &src[tokens[lo].start..tokens[hi - 1].end] }
}

/// Index of the first token of the postfix expression (`a.b[0].c(1)`, `(x)`, a literal)
/// that ends just before token `end`; `None` when what precedes is something else.
fn postfix_start(src: &str, tokens: &[Token], end: usize) -> Option<usize> {
    let mut i = end;
    loop {
        let last = i.checked_sub(1)?;
        let t = &tokens[last];
        let first = if t.is(src, ")") {
            let open = opening(src, tokens, last)?;
            match open.checked_sub(1).map(|p| &tokens[p]) {
                // A call: go on from its name.
                Some(p) if p.kind == Kind::Ident => open - 1,
                // `f(a)(b)`, `a[0](b)`, `M<T>(b)`: not a parenthesised expression.
                Some(p) if [")", "]", "}", ">"].iter().any(|c| p.is(src, c)) => return None,
                Some(p) if p.kind != Kind::Punct => return None,
                _ => return Some(open),
            }
        } else if t.is(src, "]") {
            i = opening(src, tokens, last)?;
            continue;
        } else if t.kind != Kind::Punct {
            last
        } else {
            return None;
        };
        match first.checked_sub(1).map(|p| &tokens[p]) {
            Some(p) if p.is(src, ".") || p.is(src, "?.") => i = first - 1,
            // `new Foo(…)`, `x is T y`: two words in a row.
            Some(p) if p.kind != Kind::Punct => return None,
            _ => return Some(first),
        }
    }
}

/// Whether the token range `lo..hi` is one postfix expression.
fn is_postfix(src: &str, tokens: &[Token], lo: usize, hi: usize) -> bool {
    lo < hi && postfix_start(src, &tokens[..hi], hi) == Some(lo)
}

/// An expression that can follow or precede an operator without changing meaning.
fn wrap(expression: &str) -> String {
    let simple = expression.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']'));
    if simple { expression.to_owned() } else { format!("({expression})") }
}

// --- LINQ chains -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Where,
    Select,
    SelectMany,
    Count,
    LongCount,
    Any,
    All,
    First,
    FirstOrDefault,
    Last,
    LastOrDefault,
    Single,
    SingleOrDefault,
    ElementAt,
    ElementAtOrDefault,
    Take,
    Skip,
    TakeWhile,
    SkipWhile,
    Sum,
    Min,
    Max,
    Average,
    Contains,
    Distinct,
    OrderBy,
    OrderByDescending,
    ToList,
    ToArray,
    AsEnumerable,
}

impl Op {
    fn named(name: &str) -> Option<Op> {
        Some(match name {
            "Where" => Op::Where,
            "Select" => Op::Select,
            "SelectMany" => Op::SelectMany,
            "Count" => Op::Count,
            "LongCount" => Op::LongCount,
            "Any" => Op::Any,
            "All" => Op::All,
            "First" => Op::First,
            "FirstOrDefault" => Op::FirstOrDefault,
            "Last" => Op::Last,
            "LastOrDefault" => Op::LastOrDefault,
            "Single" => Op::Single,
            "SingleOrDefault" => Op::SingleOrDefault,
            "ElementAt" => Op::ElementAt,
            "ElementAtOrDefault" => Op::ElementAtOrDefault,
            "Take" => Op::Take,
            "Skip" => Op::Skip,
            "TakeWhile" => Op::TakeWhile,
            "SkipWhile" => Op::SkipWhile,
            "Sum" => Op::Sum,
            "Min" => Op::Min,
            "Max" => Op::Max,
            "Average" => Op::Average,
            "Contains" => Op::Contains,
            "Distinct" => Op::Distinct,
            "OrderBy" => Op::OrderBy,
            "OrderByDescending" => Op::OrderByDescending,
            "ToList" => Op::ToList,
            "ToArray" => Op::ToArray,
            "AsEnumerable" => Op::AsEnumerable,
            _ => return None,
        })
    }

    /// Whether the result is a sequence, so that more calls can follow in the chain.
    fn yields_sequence(self) -> bool {
        use Op::*;
        matches!(
            self,
            Where
                | Select
                | SelectMany
                | Take
                | Skip
                | TakeWhile
                | SkipWhile
                | Distinct
                | OrderBy
                | OrderByDescending
                | ToList
                | ToArray
                | AsEnumerable
        )
    }
}

#[derive(Debug)]
struct Call<'a> {
    op: Op,
    name: &'a str,
    args: Vec<&'a str>,
}

/// `receiver.Op(…).Op(…)` inside a larger expression: bytes `start..end`.
#[derive(Debug)]
struct Chain<'a> {
    start: usize,
    end: usize,
    /// Index of the token after the chain.
    next: usize,
    receiver: &'a str,
    calls: Vec<Call<'a>>,
}

impl Chain<'_> {
    fn has_lambda(&self) -> bool {
        self.calls.iter().any(|c| c.args.iter().any(|a| a.contains("=>")))
    }
}

/// The call `.Name(args)` (or `.Name<T>(args)`) whose dot is token `dot`, with the index after it.
fn parse_call<'a>(src: &'a str, tokens: &[Token], dot: usize) -> Option<(Call<'a>, usize)> {
    if !tokens.get(dot)?.is(src, ".") {
        return None;
    }
    let name = tokens.get(dot + 1).filter(|t| t.kind == Kind::Ident)?.text(src);
    let op = Op::named(name)?;
    let mut open = dot + 2;
    if tokens.get(open)?.is(src, "<") {
        // Type arguments: names, dots, commas and brackets only.
        let mut depth = 0;
        loop {
            let t = tokens.get(open)?;
            match t.text(src) {
                "<" => depth += 1,
                ">" => depth -= 1,
                "." | "," | "?" | "[" | "]" => {}
                _ if t.kind == Kind::Ident => {}
                _ => return None,
            }
            open += 1;
            if depth == 0 {
                break;
            }
        }
    }
    if !tokens.get(open)?.is(src, "(") {
        return None;
    }
    let close = closing(src, tokens, open)?;
    let mut args = Vec::new();
    let mut from = open + 1;
    for i in top_level(src, tokens, open + 1, close) {
        if tokens[i].is(src, ",") {
            args.push(slice(src, tokens, from, i));
            from = i + 1;
        }
    }
    if from < close || !args.is_empty() {
        args.push(slice(src, tokens, from, close));
    }
    Some((Call { op, name, args }, close + 1))
}

/// The first LINQ chain that starts at or after token `from`.
fn find_chain<'a>(src: &'a str, tokens: &[Token], from: usize) -> Option<Chain<'a>> {
    for dot in from..tokens.len() {
        let Some((call, mut next)) = parse_call(src, tokens, dot) else { continue };
        let Some(first) = postfix_start(src, tokens, dot) else { continue };
        let mut calls = vec![call];
        while calls.last().is_some_and(|c| c.op.yields_sequence()) {
            let Some((call, after)) = parse_call(src, tokens, next) else { break };
            calls.push(call);
            next = after;
        }
        return Some(Chain {
            start: tokens[first].start,
            end: tokens[next - 1].end,
            next,
            receiver: slice(src, tokens, first, dot),
            calls,
        });
    }
    None
}

/// A lambda or method group passed to a LINQ method.
#[derive(Debug, PartialEq)]
enum Func {
    Lambda { params: Vec<String>, body: String },
    Group(String),
}

impl Func {
    fn parse(arg: &str) -> Result<Func, String> {
        let tokens = tokenize(arg)?;
        let top = top_level(arg, &tokens, 0, tokens.len());
        let Some(arrow) = top.iter().copied().find(|i| tokens[*i].is(arg, "=>")) else {
            let path = !tokens.is_empty()
                && tokens
                    .iter()
                    .enumerate()
                    .all(|(i, t)| if i % 2 == 0 { t.kind == Kind::Ident } else { t.is(arg, ".") })
                && tokens.len() % 2 == 1;
            return if path {
                Ok(Func::Group(arg.trim().to_owned()))
            } else {
                Err(format!("`{}` is not a lambda", arg.trim()))
            };
        };
        let head = &tokens[..arrow];
        let names: Vec<&Token> = match head {
            [name] if name.kind == Kind::Ident => vec![name],
            [open, inner @ .., close] if open.is(arg, "(") && close.is(arg, ")") => {
                let ok = !inner.is_empty()
                    && inner.iter().enumerate().all(
                        |(i, t)| {
                            if i % 2 == 0 { t.kind == Kind::Ident } else { t.is(arg, ",") }
                        },
                    )
                    && inner.len() % 2 == 1;
                if !ok {
                    return Err("only `x => …` and `(x, i) => …` lambdas are supported".into());
                }
                inner.iter().step_by(2).collect()
            }
            _ => return Err("only `x => …` and `(x, i) => …` lambdas are supported".into()),
        };
        if names.len() > 2 {
            return Err("lambdas with more than two parameters are not supported".into());
        }
        let body = arg[tokens[arrow].end..].trim();
        if body.is_empty() || body.starts_with('{') {
            return Err("statement lambdas are not supported".into());
        }
        Ok(Func::Lambda { params: names.iter().map(|t| t.text(arg).to_owned()).collect(), body: body.to_owned() })
    }

    /// `x => x`.
    fn is_identity(&self) -> bool {
        matches!(self, Func::Lambda { params, body } if params.len() == 1 && *body == params[0])
    }

    /// The expression for one element: the body with the parameters replaced.
    fn apply(&self, elem: &Elem, index: usize) -> Result<String, String> {
        match self {
            Func::Group(path) => match elem {
                Elem::One(e) => Ok(format!("{path}({e})")),
                Elem::Pair { .. } => Err("a dictionary entry cannot be passed to a method group".into()),
            },
            Func::Lambda { params, body } => {
                substitute(body, &params[0], elem, params.get(1).map(|p| (p.as_str(), index)))
            }
        }
    }
}

/// `body` with `param` replaced by the element's expression (and the index parameter
/// by its value). Member names, named arguments and literals are left alone.
fn substitute(body: &str, param: &str, elem: &Elem, index: Option<(&str, usize)>) -> Result<String, String> {
    let tokens = tokenize(body)?;
    let mut out = String::with_capacity(body.len() + 16);
    let mut pos = 0;
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        let prev = i.checked_sub(1).map(|p| &tokens[p]);
        let next = tokens.get(i + 1);
        let member = prev.is_some_and(|p| [".", "?.", "::", "->"].iter().any(|d| p.is(body, d)));
        let named_argument =
            next.is_some_and(|n| n.is(body, ":")) && prev.is_some_and(|p| p.is(body, "(") || p.is(body, ","));
        if t.kind != Kind::Ident || member || named_argument {
            i += 1;
            continue;
        }
        let name = t.text(body);
        if name == param {
            out.push_str(&body[pos..t.start]);
            match elem {
                Elem::One(e) => {
                    out.push_str(&wrap(e));
                    pos = t.end;
                }
                Elem::Pair { key, value } => {
                    let field =
                        tokens.get(i + 2).filter(|_| next.is_some_and(|n| n.is(body, "."))).map(|f| f.text(body));
                    let replacement = match field {
                        Some("Key") => key,
                        Some("Value") => value,
                        _ => {
                            return Err(format!(
                                "a dictionary entry can only be used as `{param}.Key` or `{param}.Value`"
                            ));
                        }
                    };
                    out.push_str(&wrap(replacement));
                    pos = tokens[i + 2].end;
                    i += 2;
                }
            }
        } else if let Some((_, n)) = index.filter(|(p, _)| *p == name) {
            out.push_str(&body[pos..t.start]);
            out.push_str(&n.to_string());
            pos = t.end;
        }
        i += 1;
    }
    out.push_str(&body[pos..]);
    Ok(out)
}

/// How one element is written in an expression the adapter evaluates.
#[derive(Clone, Debug, PartialEq)]
enum Elem {
    One(String),
    /// A dictionary entry: there is no expression for the pair itself.
    Pair {
        key: String,
        value: String,
    },
}

#[derive(Clone, Debug)]
struct Item {
    elem: Elem,
    /// The element's value, once read.
    var: Option<Variable>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Enumerable,
    List,
    Array,
}

struct Seq {
    items: Vec<Item>,
    /// Declared element type, when the collection's type tells it.
    element: Option<String>,
    /// `KeyValuePair<K, V>` of a dictionary's entries.
    pair: Option<String>,
    shape: Shape,
}

/// The result of a chain: its value, and the expression that stands for it inside a
/// larger expression (none for a sequence).
struct Answer {
    var: Variable,
    splice: Option<String>,
}

fn value(value: impl Into<String>, type_name: &str) -> Variable {
    Variable {
        name: String::new(),
        value: value.into(),
        type_name: Some(type_name.to_owned()).filter(|t| !t.is_empty()),
        variables_reference: 0,
        evaluate_name: None,
    }
}

fn boolean(b: bool) -> Variable {
    value(if b { "true" } else { "false" }, "bool")
}

fn is_null(v: &Variable) -> bool {
    v.value == "null"
}

fn type_of(v: &Variable) -> &str {
    v.type_name.as_deref().unwrap_or_default()
}

fn as_bool(v: &Variable) -> Result<bool, String> {
    match (type_of(v), v.value.as_str()) {
        ("bool", "true") => Ok(true),
        ("bool", "false") => Ok(false),
        (ty, _) if ty.ends_with("Exception") => Err(format!("the condition threw {ty}")),
        (ty, _) => Err(format!("the condition is a `{ty}`, not a bool")),
    }
}

/// Element count of an array from netcoredbg's `{int[3]}`.
fn array_len(type_name: &str, value: &str) -> Option<usize> {
    let element = type_name.strip_suffix("[]")?;
    value.strip_prefix('{')?.strip_prefix(element)?.strip_prefix('[')?.strip_suffix("]}")?.parse().ok()
}

fn is_index(name: &str, index: usize) -> bool {
    name.strip_prefix('[').and_then(|n| n.strip_suffix(']')).and_then(|n| n.parse::<usize>().ok()) == Some(index)
}

/// An exact decimal number as printed: sign, integer digits and fraction digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Dec {
    negative: bool,
    whole: String,
    fraction: String,
}

impl Dec {
    fn parse(text: &str) -> Option<Dec> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(d) => (true, d),
            None => (false, text),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) {
            return None;
        }
        let whole = whole.trim_start_matches('0').to_owned();
        let fraction = fraction.trim_end_matches('0').to_owned();
        Some(Dec { negative: negative && !(whole.is_empty() && fraction.is_empty()), whole, fraction })
    }

    fn magnitude(&self, other: &Dec) -> Ordering {
        self.whole
            .len()
            .cmp(&other.whole.len())
            .then_with(|| self.whole.cmp(&other.whole))
            .then_with(|| self.fraction.cmp(&other.fraction))
    }
}

impl Ord for Dec {
    fn cmp(&self, other: &Dec) -> Ordering {
        match (self.negative, other.negative) {
            (false, false) => self.magnitude(other),
            (true, true) => other.magnitude(self),
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
        }
    }
}

impl PartialOrd for Dec {
    fn partial_cmp(&self, other: &Dec) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A value this side can compare exactly, from its printed form.
#[derive(Clone, Debug, PartialEq)]
enum Key {
    Null,
    Bool(bool),
    Int(i128),
    Dec(Dec),
    /// Printed with 16 digits: two different doubles can read the same.
    Float(f64),
    Char(u32),
    Str(String),
    Other,
}

const INT_TYPES: &[&str] = &["int", "long", "short", "sbyte", "byte", "ushort", "uint", "ulong"];

fn key_of(v: &Variable) -> Key {
    if is_null(v) {
        return Key::Null;
    }
    let ty = type_of(v).trim_end_matches('?');
    let parsed = match ty {
        "bool" => v.value.parse().ok().map(Key::Bool),
        "decimal" => Dec::parse(&v.value).map(Key::Dec),
        "double" | "float" => v.value.parse::<f64>().ok().filter(|f| f.is_finite()).map(Key::Float),
        // `97 'a'`
        "char" => v.value.split(' ').next().and_then(|n| n.parse().ok()).map(Key::Char),
        "string" => Some(Key::Str(v.value.clone())),
        _ if INT_TYPES.contains(&ty) => v.value.parse().ok().map(Key::Int),
        _ => None,
    };
    parsed.unwrap_or(Key::Other)
}

impl Key {
    fn dec(&self) -> Option<Dec> {
        match self {
            Key::Int(i) => Dec::parse(&i.to_string()),
            Key::Dec(d) => Some(d.clone()),
            _ => None,
        }
    }

    /// A string equal for equal values, for the kinds compared here.
    fn canonical(&self) -> Option<String> {
        Some(match self {
            Key::Null => "null".to_owned(),
            Key::Bool(b) => format!("b{b}"),
            Key::Int(_) | Key::Dec(_) => {
                let d = self.dec()?;
                format!("n{}{}.{}", if d.negative { "-" } else { "" }, d.whole, d.fraction)
            }
            Key::Char(c) => format!("c{c}"),
            Key::Str(s) => format!("s{s}"),
            Key::Float(_) | Key::Other => return None,
        })
    }

    fn numeric(&self) -> bool {
        matches!(self, Key::Int(_) | Key::Dec(_) | Key::Float(_))
    }
}

/// An element ready to be compared: its key and the expression the adapter can use.
#[derive(Clone)]
struct Keyed {
    key: Key,
    expr: String,
    type_name: String,
}

/// The type a missing element defaults to, as a value and as a literal.
fn primitive_default(ty: &str) -> Option<(&'static str, &'static str)> {
    Some(match ty {
        "bool" => ("false", "false"),
        "int" | "short" | "sbyte" | "byte" | "ushort" => ("0", "0"),
        "long" => ("0", "0L"),
        "uint" => ("0", "0u"),
        "ulong" => ("0", "0ul"),
        "decimal" => ("0", "0m"),
        "double" => ("0", "0.0"),
        "float" => ("0", "0f"),
        "char" => ("0 '\\0'", "'\\0'"),
        _ => return None,
    })
}

fn is_reference_type(ty: &str) -> bool {
    ty == "string" || ty == "object" || ty.ends_with('?') || ty.ends_with(']')
}

const ALIASES: &[(&str, &str)] = &[
    ("bool", "System.Boolean"),
    ("byte", "System.Byte"),
    ("sbyte", "System.SByte"),
    ("char", "System.Char"),
    ("decimal", "System.Decimal"),
    ("double", "System.Double"),
    ("float", "System.Single"),
    ("int", "System.Int32"),
    ("uint", "System.UInt32"),
    ("long", "System.Int64"),
    ("ulong", "System.UInt64"),
    ("short", "System.Int16"),
    ("ushort", "System.UInt16"),
    ("object", "System.Object"),
    ("string", "System.String"),
];

struct Evaluator<'h> {
    host: &'h dyn Host,
    deadline: Instant,
    depth: usize,
    /// Names used as `Name.member`: the type they stand for, or `None` for a value.
    types: HashMap<String, Option<String>>,
    namespaces: Option<Vec<String>>,
}

impl<'h> Evaluator<'h> {
    fn new(host: &'h dyn Host) -> Self {
        Self { host, deadline: Instant::now() + TIME_BUDGET, depth: 0, types: HashMap::new(), namespaces: None }
    }

    fn tick(&self) -> Outcome<()> {
        if self.host.cancelled() {
            return Err(Fail::Fatal(IdeError::cancelled()));
        }
        if Instant::now() > self.deadline {
            return error(format!(
                "the evaluation took longer than {} s and was stopped; try a smaller collection or a simpler expression",
                TIME_BUDGET.as_secs()
            ));
        }
        Ok(())
    }

    /// The adapter's own answer: `Err` with its message when it refuses.
    async fn native(&mut self, expression: &str) -> Outcome<Result<Variable, String>> {
        self.tick()?;
        match self.host.evaluate(expression).await {
            Ok(v) => Ok(Ok(v)),
            Err(DapError::Failed { message, .. }) => Ok(Err(message)),
            Err(e) => Err(Fail::Fatal(e.into())),
        }
    }

    /// The adapter's answer for an expression built here.
    async fn plain(&mut self, expression: &str) -> Outcome<Variable> {
        self.native(expression).await?.map_err(Fail::Error)
    }

    async fn children(&mut self, reference: i64, count: Option<usize>) -> Outcome<Vec<Variable>> {
        self.tick()?;
        match self.host.variables(reference, count).await {
            Ok(v) => Ok(v),
            Err(DapError::Failed { message, .. }) => error(message),
            Err(e) => Err(Fail::Fatal(e.into())),
        }
    }

    /// The adapter's answer, or this module's when the adapter has none.
    fn eval(&mut self, expression: String) -> BoxFuture<'_, Outcome<Variable>> {
        Box::pin(async move {
            let native = self.native(&expression).await?;
            if let Ok(v) = &native
                && !suspicious(v)
            {
                return Ok(v.clone());
            }
            if self.depth >= MAX_DEPTH {
                return native.map_err(Fail::Error);
            }
            self.depth += 1;
            let helped = self.help(&expression, &native).await;
            self.depth -= 1;
            match (helped, native) {
                (Ok(v), _) => Ok(v),
                (Err(Fail::Fatal(e)), _) => Err(Fail::Fatal(e)),
                (Err(_), Ok(v)) => Ok(v),
                (Err(Fail::Skip(_)), Err(message)) => error(message),
                (Err(e), Err(_)) => Err(e),
            }
        })
    }

    async fn help(&mut self, expression: &str, native: &Result<Variable, String>) -> Outcome<Variable> {
        let tokens = tokenize(expression).map_err(Fail::Skip)?;
        if native.is_err()
            && let Some(v) = self.logic(expression, &tokens).await?
        {
            return Ok(v);
        }
        let reason = match self.chains(expression, &tokens).await {
            Err(Fail::Skip(reason)) => reason,
            other => return other,
        };
        if native.is_ok() {
            return Err(Fail::Skip(reason));
        }
        if let Some(v) = self.lengths(expression, &tokens).await? {
            return Ok(v);
        }
        if let Some(v) = self.qualified(expression, &tokens).await? {
            return Ok(v);
        }
        Err(Fail::Skip(reason))
    }

    // --- `&&`, `||`, `!` and null tests --------------------------------------------------

    /// Short-circuits `&&` / `||` and answers `x == null` from `x`'s value: netcoredbg
    /// evaluates both operands and has no null comparison for references.
    async fn logic(&mut self, src: &str, tokens: &[Token]) -> Outcome<Option<Variable>> {
        let (mut lo, mut hi) = (0, tokens.len());
        while hi > lo + 1 && tokens[lo].is(src, "(") && closing(src, tokens, lo) == Some(hi - 1) {
            lo += 1;
            hi -= 1;
        }
        let top = top_level(src, tokens, lo, hi);
        let lower_precedence = |t: &Token| {
            let text = t.text(src);
            t.kind == Kind::Punct && (["?", "??", "=>", ":", ","].contains(&text) || ASSIGNMENTS.contains(&text))
        };
        if top.iter().any(|i| lower_precedence(&tokens[*i])) {
            return Ok(None);
        }
        for op in ["||", "&&"] {
            let at: Vec<usize> = top.iter().copied().filter(|i| tokens[*i].is(src, op)).collect();
            if at.is_empty() {
                continue;
            }
            let decides = op == "||";
            let mut from = lo;
            for end in at.into_iter().chain([hi]) {
                let part = slice(src, tokens, from, end);
                if part.is_empty() {
                    return Ok(None);
                }
                let v = self.eval(part.to_owned()).await?;
                if as_bool(&v).map_err(Fail::Error)? == decides {
                    return Ok(Some(boolean(decides)));
                }
                from = end + 1;
            }
            return Ok(Some(boolean(!decides)));
        }
        if tokens.get(lo).is_some_and(|t| t.is(src, "!")) && is_postfix(src, tokens, lo + 1, hi) {
            let v = self.eval(slice(src, tokens, lo + 1, hi).to_owned()).await?;
            return Ok(Some(boolean(!as_bool(&v).map_err(Fail::Error)?)));
        }
        let Some((from, to, negated)) = null_test(src, tokens, lo, hi) else { return Ok(None) };
        let v = self.eval(slice(src, tokens, from, to).to_owned()).await?;
        Ok(Some(boolean(is_null(&v) != negated)))
    }

    // --- LINQ ----------------------------------------------------------------------------

    /// Runs the first LINQ chain the adapter cannot, and evaluates the expression around
    /// it with the chain replaced by its result.
    async fn chains(&mut self, src: &str, tokens: &[Token]) -> Outcome<Variable> {
        let mut from = 0;
        let mut reason = String::new();
        while let Some(chain) = find_chain(src, tokens, from) {
            from = chain.next;
            let whole = src[..chain.start].trim().is_empty() && src[chain.end..].trim().is_empty();
            if !whole
                && !chain.has_lambda()
                && let Ok(v) = self.native(&src[chain.start..chain.end]).await?
                && !suspicious(&v)
            {
                continue;
            }
            match self.run(&chain).await {
                Ok(answer) if whole => return Ok(answer.var),
                Ok(Answer { splice: Some(splice), .. }) => {
                    let rewritten = format!("{}({}){}", &src[..chain.start], splice, &src[chain.end..]);
                    return self.eval(rewritten).await;
                }
                Ok(_) => {
                    let dot = tokens.get(chain.next).is_some_and(|t| t.is(src, "."));
                    let after = tokens.get(chain.next + 1).filter(|_| dot);
                    return error(match after {
                        Some(name) => {
                            format!("`{}` cannot follow a LINQ sequence here; supported: {SUPPORTED}", name.text(src))
                        }
                        None => "a LINQ sequence cannot be used inside a larger expression".to_owned(),
                    });
                }
                Err(Fail::Skip(r)) => {
                    if reason.is_empty() {
                        reason = r;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(Fail::Skip(reason))
    }

    async fn run(&mut self, chain: &Chain<'_>) -> Outcome<Answer> {
        let mut seq = self.enumerate(chain.receiver.to_owned()).await?;
        for call in &chain.calls {
            match self.apply(seq, call).await? {
                Step::Seq(next) => seq = next,
                Step::Value(answer) => return Ok(answer),
            }
        }
        self.sequence(seq).await
    }

    /// The elements of the collection `expression` evaluates to.
    async fn enumerate(&mut self, expression: String) -> Outcome<Seq> {
        let v = match self.eval(expression.clone()).await {
            Ok(v) => v,
            Err(Fail::Error(message)) => return Err(Fail::Skip(message)),
            Err(e) => return Err(e),
        };
        if is_null(&v) {
            return error(format!("`{expression}` is null"));
        }
        self.shape(expression, v).await
    }

    fn too_large<T>(expression: &str, count: usize) -> Outcome<T> {
        error(format!("`{expression}` has {count} elements; LINQ evaluation handles up to {MAX_ELEMENTS}"))
    }

    /// Reads the elements out of the collection's fields, as the runtime lays them out.
    fn shape(&mut self, expression: String, v: Variable) -> BoxFuture<'_, Outcome<Seq>> {
        Box::pin(async move {
            let ty = type_of(&v).to_owned();
            let indexed = |children: Vec<Variable>, container: &str| -> Option<Vec<Item>> {
                children
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let expr = c.evaluate_name.clone().unwrap_or_else(|| format!("{}{}", wrap(container), c.name));
                        is_index(&c.name, i).then_some(Item { elem: Elem::One(expr), var: Some(c) })
                    })
                    .collect()
            };
            if let Some(n) = array_len(&ty, &v.value) {
                if n > MAX_ELEMENTS {
                    return Self::too_large(&expression, n);
                }
                let children = if n > 0 && v.variables_reference > 0 {
                    self.children(v.variables_reference, Some(n)).await?
                } else {
                    vec![]
                };
                let Some(items) = indexed(children, &expression) else {
                    return Err(Fail::Skip(format!("`{expression}` is not a one-dimensional array")));
                };
                let element = ty.strip_suffix("[]").map(str::to_owned);
                return Ok(Seq { items, element, pair: None, shape: Shape::Enumerable });
            }
            if v.variables_reference == 0 {
                return Err(Fail::Skip(format!("`{expression}` is not a collection")));
            }
            let fields = self.children(v.variables_reference, None).await?;
            let field = |name: &str| fields.iter().find(|f| f.name == name);
            let count = |f: &Variable| f.value.parse::<usize>().ok();

            // List<T>: `_items` holds `_size` elements and spare capacity.
            if let (Some(items), Some(size)) = (field("_items"), field("_size").and_then(count)) {
                if size > MAX_ELEMENTS {
                    return Self::too_large(&expression, size);
                }
                let mut children = if size > 0 && items.variables_reference > 0 {
                    self.children(items.variables_reference, Some(size)).await?
                } else {
                    vec![]
                };
                children.truncate(size);
                let container = items.evaluate_name.clone().unwrap_or_else(|| format!("{}._items", wrap(&expression)));
                let items = indexed(children, &container)
                    .filter(|i| i.len() == size)
                    .ok_or_else(|| Fail::Error(format!("the elements of `{expression}` could not be read")))?;
                let element = type_of(field("_items").unwrap_or(&v)).strip_suffix("[]").map(str::to_owned);
                return Ok(Seq { items, element, pair: None, shape: Shape::Enumerable });
            }

            // Dictionary<K, V> and HashSet<T>: `_entries` holds `_count` slots, the free
            // ones marked by `next < -1`.
            if let (Some(entries), Some(used), Some(_)) =
                (field("_entries"), field("_count").and_then(count), field("_buckets"))
            {
                if used > MAX_ELEMENTS {
                    return Self::too_large(&expression, used);
                }
                let mut slots = if used > 0 && entries.variables_reference > 0 {
                    self.children(entries.variables_reference, Some(used)).await?
                } else {
                    vec![]
                };
                slots.truncate(used);
                let mut items = Vec::with_capacity(slots.len());
                let mut pairs = false;
                for slot in slots {
                    let parts = self.children(slot.variables_reference, None).await?;
                    let part = |names: [&str; 2]| parts.iter().find(|p| names.contains(&p.name.as_str()));
                    let unknown = || Fail::Skip(format!("the entries of `{expression}` have an unknown layout"));
                    let next = part(["next", "Next"]).and_then(|n| n.value.parse::<i64>().ok()).ok_or_else(unknown)?;
                    if next < -1 {
                        continue;
                    }
                    let path = |p: &Variable| p.evaluate_name.clone().ok_or_else(unknown);
                    match (part(["key", "Key"]), part(["value", "Value"])) {
                        (Some(k), Some(val)) => {
                            pairs = true;
                            items.push(Item { elem: Elem::Pair { key: path(k)?, value: path(val)? }, var: None });
                        }
                        (None, Some(val)) => items.push(Item { elem: Elem::One(path(val)?), var: Some(val.clone()) }),
                        _ => return Err(unknown()),
                    }
                }
                let arguments = ty.find('<').map(|at| &ty[at..]).unwrap_or_default();
                let pair = pairs.then(|| format!("System.Collections.Generic.KeyValuePair{arguments}"));
                return Ok(Seq { items, element: None, pair, shape: Shape::Enumerable });
            }

            // `dict.Keys` / `dict.Values`.
            let keys = ty.ends_with(".KeyCollection");
            if let Some(dictionary) = field("_dictionary").filter(|_| keys || ty.ends_with(".ValueCollection")) {
                let path =
                    dictionary.evaluate_name.clone().unwrap_or_else(|| format!("{}._dictionary", wrap(&expression)));
                let mut seq = self.shape(path, dictionary.clone()).await?;
                for item in &mut seq.items {
                    if let Elem::Pair { key, value } = &item.elem {
                        item.elem = Elem::One(if keys { key.clone() } else { value.clone() });
                    }
                }
                seq.pair = None;
                return Ok(seq);
            }

            // A sequence result of this module.
            if Synthetic::is(v.variables_reference)
                && let Some(items) = indexed(fields.clone(), &expression)
            {
                return Ok(Seq { items, element: None, pair: None, shape: Shape::Enumerable });
            }

            // Anything else that can copy itself into an array (Queue<T>, a LINQ iterator…).
            let copy = format!("{}.ToArray()", wrap(&expression));
            if !expression.ends_with(".ToArray()")
                && let Ok(array) = self.native(&copy).await?
                && array_len(type_of(&array), &array.value).is_some()
            {
                return self.shape(copy, array).await;
            }
            Err(Fail::Skip(format!(
                "`{expression}` is a `{ty}`; LINQ evaluation needs an array, List<T>, Dictionary<K, V>, HashSet<T> \
                 or a collection with ToArray()"
            )))
        })
    }

    /// The element's value (read once).
    async fn var_of(&mut self, item: &mut Item, pair: Option<&str>) -> Outcome<Variable> {
        if let Some(v) = &item.var {
            return Ok(v.clone());
        }
        let v = match &item.elem {
            Elem::One(e) => self.eval(e.clone()).await?,
            Elem::Pair { key, value: val } => {
                let mut k = self.eval(key.clone()).await?;
                let mut v = self.eval(val.clone()).await?;
                let text = format!("[{}, {}]", k.value, v.value);
                (k.name, k.evaluate_name) = ("Key".into(), Some(key.clone()));
                (v.name, v.evaluate_name) = ("Value".into(), Some(val.clone()));
                Variable { variables_reference: self.host.keep(vec![k, v]), ..value(text, pair.unwrap_or_default()) }
            }
        };
        item.var = Some(v.clone());
        Ok(v)
    }

    async fn keyed(&mut self, item: &mut Item) -> Outcome<Keyed> {
        let Elem::One(expr) = &item.elem else {
            return error("dictionary entries cannot be compared; select `.Key` or `.Value` first");
        };
        let expr = wrap(expr);
        let v = self.var_of(item, None).await?;
        Ok(Keyed { key: key_of(&v), expr, type_name: type_of(&v).to_owned() })
    }

    /// The lambda's value for one element.
    async fn call(&mut self, f: &Func, item: &mut Item, index: usize) -> Outcome<Item> {
        if f.is_identity() {
            return Ok(item.clone());
        }
        let text = f.apply(&item.elem, index).map_err(Fail::Error)?;
        let v = self.eval(text.clone()).await.map_err(|e| at_element(e, index))?;
        Ok(Item { elem: Elem::One(text), var: Some(v) })
    }

    async fn test(&mut self, f: &Func, item: &Item, index: usize) -> Outcome<bool> {
        let text = f.apply(&item.elem, index).map_err(Fail::Error)?;
        let v = self.eval(text).await.map_err(|e| at_element(e, index))?;
        as_bool(&v).map_err(|m| at_element(Fail::Error(m), index))
    }

    async fn filter(&mut self, items: Vec<Item>, f: &Func) -> Outcome<Vec<Item>> {
        let mut kept = Vec::new();
        for (i, item) in items.into_iter().enumerate() {
            if self.test(f, &item, i).await? {
                kept.push(item);
            }
        }
        Ok(kept)
    }

    async fn project(&mut self, seq: Seq, f: &Func) -> Outcome<Seq> {
        if f.is_identity() {
            return Ok(seq);
        }
        let mut items = Vec::with_capacity(seq.items.len());
        for (i, mut item) in seq.items.into_iter().enumerate() {
            items.push(self.call(f, &mut item, i).await?);
        }
        Ok(Seq { items, element: None, pair: None, shape: Shape::Enumerable })
    }

    async fn integer(&mut self, argument: &str) -> Outcome<usize> {
        let v = self.eval(argument.to_owned()).await?;
        match key_of(&v) {
            Key::Int(n) => Ok(usize::try_from(n.max(0)).unwrap_or(usize::MAX)),
            _ => error(format!("`{argument}` is not an integer")),
        }
    }

    async fn apply(&mut self, mut seq: Seq, call: &Call<'_>) -> Outcome<Step> {
        use Op::*;
        let name = call.name;
        let arity = |min: usize, max: usize| -> Outcome<()> {
            if (min..=max).contains(&call.args.len()) {
                return Ok(());
            }
            error(match (min, max) {
                (0, 0) => format!("{name}() takes no arguments here"),
                (1, 1) => format!("{name}() takes one argument here"),
                _ => format!("{name}() takes {min} to {max} arguments here"),
            })
        };
        let func = |at: usize| -> Outcome<Option<Func>> {
            call.args.get(at).map(|a| Func::parse(a).map_err(|m| Fail::Error(format!("{name}: {m}")))).transpose()
        };
        let required = |f: Option<Func>| f.ok_or_else(|| Fail::Error(format!("{name}() needs a lambda")));
        let scalar =
            |var: Variable, splice: String| -> Outcome<Step> { Ok(Step::Value(Answer { var, splice: Some(splice) })) };
        match call.op {
            Where => {
                arity(1, 1)?;
                seq.items = self.filter(seq.items, &required(func(0)?)?).await?;
            }
            Select => {
                arity(1, 1)?;
                seq = self.project(seq, &required(func(0)?)?).await?;
            }
            SelectMany => {
                arity(1, 1)?;
                let f = required(func(0)?)?;
                let mut items = Vec::new();
                for (i, item) in seq.items.iter().enumerate() {
                    let text = f.apply(&item.elem, i).map_err(Fail::Error)?;
                    let inner = match self.enumerate(text).await {
                        Err(Fail::Skip(reason)) => Err(Fail::Error(reason)),
                        other => other,
                    };
                    items.extend(inner.map_err(|e| at_element(e, i))?.items);
                    if items.len() > MAX_ELEMENTS {
                        return Self::too_large(call.name, items.len());
                    }
                }
                seq = Seq { items, element: None, pair: None, shape: Shape::Enumerable };
            }
            Take | Skip => {
                arity(1, 1)?;
                let n = self.integer(call.args[0]).await?.min(seq.items.len());
                if call.op == Take {
                    seq.items.truncate(n);
                } else {
                    seq.items.drain(..n);
                }
            }
            TakeWhile | SkipWhile => {
                arity(1, 1)?;
                let f = required(func(0)?)?;
                let mut n = 0;
                while n < seq.items.len() && self.test(&f, &seq.items[n], n).await? {
                    n += 1;
                }
                if call.op == TakeWhile {
                    seq.items.truncate(n);
                } else {
                    seq.items.drain(..n);
                }
            }
            Distinct => {
                arity(0, 0)?;
                seq.items = self.distinct(seq.items).await?;
            }
            OrderBy | OrderByDescending => {
                arity(1, 1)?;
                let f = required(func(0)?)?;
                let mut keys = Vec::with_capacity(seq.items.len());
                for (i, item) in seq.items.iter_mut().enumerate() {
                    let mut key = self.call(&f, item, i).await?;
                    keys.push(self.keyed(&mut key).await?);
                }
                let order = self.sort(&keys, call.op == OrderByDescending).await?;
                let mut items: Vec<Option<Item>> = seq.items.into_iter().map(Some).collect();
                seq.items = order.into_iter().filter_map(|i| items[i].take()).collect();
            }
            ToList | ToArray | AsEnumerable => {
                arity(0, 0)?;
                seq.shape = match call.op {
                    ToList => Shape::List,
                    ToArray => Shape::Array,
                    _ => seq.shape,
                };
            }
            Count | LongCount => {
                arity(0, 1)?;
                let n = match func(0)? {
                    Some(f) => self.filter(seq.items, &f).await?.len(),
                    None => seq.items.len(),
                };
                let long = call.op == LongCount;
                return scalar(
                    value(n.to_string(), if long { "long" } else { "int" }),
                    format!("{n}{}", if long { "L" } else { "" }),
                );
            }
            Any | All => {
                let all = call.op == All;
                arity(usize::from(all), 1)?;
                let found = match func(0)? {
                    None => !seq.items.is_empty(),
                    Some(f) => {
                        // Any stops at the first match, All at the first miss.
                        let mut stopped = false;
                        for (i, item) in seq.items.iter().enumerate() {
                            if self.test(&f, item, i).await? != all {
                                stopped = true;
                                break;
                            }
                        }
                        stopped != all
                    }
                };
                return scalar(boolean(found), found.to_string());
            }
            First | FirstOrDefault | Last | LastOrDefault | Single | SingleOrDefault => {
                arity(0, 1)?;
                let or_default = matches!(call.op, FirstOrDefault | LastOrDefault | SingleOrDefault);
                let f = func(0)?;
                let source = Seq {
                    items: seq.items.clone(),
                    element: seq.element.clone(),
                    pair: seq.pair.clone(),
                    shape: seq.shape,
                };
                let mut matches = Vec::new();
                match (&f, call.op) {
                    (Some(f), First | FirstOrDefault) => {
                        for (i, item) in seq.items.into_iter().enumerate() {
                            if self.test(f, &item, i).await? {
                                matches.push(item);
                                break;
                            }
                        }
                    }
                    (Some(f), _) => matches = self.filter(seq.items, f).await?,
                    (None, _) => matches = seq.items,
                }
                let single = matches!(call.op, Single | SingleOrDefault);
                if single && matches.len() > 1 {
                    return error(match f {
                        Some(_) => "Sequence contains more than one matching element",
                        None => "Sequence contains more than one element",
                    });
                }
                let picked =
                    if matches!(call.op, Last | LastOrDefault) { matches.pop() } else { matches.into_iter().next() };
                return match picked {
                    Some(item) => self.element(item, source.pair.as_deref()).await.map(Step::Value),
                    None if or_default => self.default_of(source).await.map(Step::Value),
                    None => error(match f {
                        Some(_) => "Sequence contains no matching element",
                        None => "Sequence contains no elements",
                    }),
                };
            }
            ElementAt | ElementAtOrDefault => {
                arity(1, 1)?;
                let v = self.eval(call.args[0].to_owned()).await?;
                let Key::Int(index) = key_of(&v) else {
                    return error(format!("`{}` is not an integer", call.args[0]));
                };
                let at = usize::try_from(index).ok().filter(|i| *i < seq.items.len());
                return match at {
                    Some(i) => {
                        let item = seq.items.swap_remove(i);
                        self.element(item, seq.pair.as_deref()).await.map(Step::Value)
                    }
                    None if call.op == ElementAtOrDefault => self.default_of(seq).await.map(Step::Value),
                    None => {
                        error("Index was out of range. Must be non-negative and less than the size of the collection")
                    }
                };
            }
            Sum | Average | Min | Max => {
                arity(0, 1)?;
                if let Some(f) = func(0)? {
                    seq = self.project(seq, &f).await?;
                }
                let answer = match call.op {
                    Sum => self.sum(seq).await?,
                    Average => self.average(seq).await?,
                    _ => self.extreme(seq, call.op == Max).await?,
                };
                return Ok(Step::Value(answer));
            }
            Contains => {
                arity(1, 1)?;
                let wanted = self.eval(call.args[0].to_owned()).await?;
                let wanted = Keyed {
                    key: key_of(&wanted),
                    expr: wrap(call.args[0].trim()),
                    type_name: type_of(&wanted).to_owned(),
                };
                let mut found = false;
                for item in &mut seq.items {
                    let candidate = self.keyed(item).await?;
                    if self.same(&candidate, &wanted).await? {
                        found = true;
                        break;
                    }
                }
                return scalar(boolean(found), found.to_string());
            }
        }
        Ok(Step::Seq(seq))
    }

    /// One element as the result.
    async fn element(&mut self, mut item: Item, pair: Option<&str>) -> Outcome<Answer> {
        let var = self.var_of(&mut item, pair).await?;
        let splice = match item.elem {
            Elem::One(e) => Some(e),
            Elem::Pair { .. } => None,
        };
        Ok(Answer { var, splice })
    }

    /// `default(T)` for the elements of `source`, when it can be told what that is.
    async fn default_of(&mut self, mut source: Seq) -> Outcome<Answer> {
        if source.pair.is_some() {
            return error("no element was found, and the default KeyValuePair cannot be shown");
        }
        let mut ty = source.element.clone();
        let mut sample = None;
        for item in &mut source.items {
            let v = self.var_of(item, None).await?;
            ty = ty.or_else(|| v.type_name.clone());
            if is_null(&v) {
                // Only a reference (or nullable) type holds null.
                return Ok(Answer {
                    var: value("null", ty.as_deref().unwrap_or_default()),
                    splice: Some("null".into()),
                });
            }
            if let Elem::One(e) = &item.elem {
                sample.get_or_insert_with(|| wrap(e));
            }
        }
        let Some(ty) = ty else {
            return error("no element was found, and the element type (so its default value) is not known");
        };
        if let Some((text, literal)) = primitive_default(&ty) {
            return Ok(Answer { var: value(text, &ty), splice: Some(literal.to_owned()) });
        }
        let null = Answer { var: value("null", &ty), splice: Some("null".into()) };
        if is_reference_type(&ty) {
            return Ok(null);
        }
        if let Some(sample) = sample {
            let is_value = self.plain(&format!("{sample}.GetType().IsValueType")).await?;
            if as_bool(&is_value).map_err(Fail::Error)? {
                return error(format!("no element was found, and the default `{ty}` cannot be shown"));
            }
            return Ok(null);
        }
        error(format!("the sequence is empty, and whether `{ty}` defaults to null is not known"))
    }

    /// The numeric elements with their types, nulls left out (as `Sum` and `Average` do).
    async fn numbers(&mut self, seq: &mut Seq, what: &str) -> Outcome<Vec<(Keyed, Variable)>> {
        let mut out = Vec::with_capacity(seq.items.len());
        for item in &mut seq.items {
            let keyed = self.keyed(item).await?;
            let v = self.var_of(item, None).await?;
            match keyed.key {
                Key::Null => {}
                Key::Int(_) | Key::Dec(_) | Key::Float(_) => out.push((keyed, v)),
                _ => return error(format!("{what} needs numbers; an element is a `{}`", keyed.type_name)),
            }
        }
        Ok(out)
    }

    async fn sum(&mut self, mut seq: Seq) -> Outcome<Answer> {
        let numbers = self.numbers(&mut seq, "Sum").await?;
        match self.total(&numbers, "Sum").await? {
            Total::Empty => {
                let numeric = |t: &&str| INT_TYPES.contains(t) || ["decimal", "double"].contains(t);
                let ty = seq.element.as_deref().filter(numeric).unwrap_or_default();
                Ok(Answer { var: value("0", ty), splice: Some("0".into()) })
            }
            Total::Int { value: total, long } => {
                let (min, max) =
                    if long { (i64::MIN as i128, i64::MAX as i128) } else { (i32::MIN as i128, i32::MAX as i128) };
                if total < min || total > max {
                    return error("Arithmetic operation resulted in an overflow");
                }
                Ok(Answer {
                    var: value(total.to_string(), if long { "long" } else { "int" }),
                    splice: Some(format!("{total}{}", if long { "L" } else { "" })),
                })
            }
            Total::Decimal(var) => {
                let splice = Some(format!("{}m", var.value));
                Ok(Answer { var, splice })
            }
            Total::Double { var, expression } => Ok(Answer { var, splice: Some(expression) }),
        }
    }

    async fn average(&mut self, mut seq: Seq) -> Outcome<Answer> {
        let numbers = self.numbers(&mut seq, "Average").await?;
        let n = numbers.len();
        let expression = match self.total(&numbers, "Average").await? {
            Total::Empty => return error("Sequence contains no elements"),
            Total::Int { value, .. } => format!("{value}.0 / {n}.0"),
            Total::Decimal(sum) => format!("{}m / {n}m", sum.value),
            Total::Double { expression, .. } => format!("({expression}) / {n}.0"),
        };
        let var = self.plain(&expression).await?;
        Ok(Answer { var, splice: Some(expression) })
    }

    /// The exact sum: integers are added here, decimals by the adapter from their
    /// literals, doubles by the adapter from the elements' own expressions (their
    /// printed form is rounded).
    async fn total(&mut self, numbers: &[(Keyed, Variable)], what: &str) -> Outcome<Total> {
        let types: HashSet<&str> = numbers.iter().map(|(k, _)| k.type_name.trim_end_matches('?')).collect();
        if numbers.is_empty() {
            return Ok(Total::Empty);
        }
        let only = |allowed: &[&str]| types.iter().all(|t| allowed.contains(t));
        if only(&["int", "short", "sbyte", "byte", "ushort", "long", "uint"]) {
            let value = numbers.iter().filter_map(|(k, _)| if let Key::Int(i) = k.key { Some(i) } else { None }).sum();
            return Ok(Total::Int { value, long: types.contains("long") || types.contains("uint") });
        }
        if only(&["decimal"]) {
            let mut total: Option<Variable> = None;
            for chunk in numbers.chunks(MAX_TERMS) {
                let mut terms: Vec<String> = total.iter().map(|t| format!("{}m", t.value)).collect();
                terms.extend(chunk.iter().map(|(_, v)| format!("{}m", v.value)));
                let sum = self.plain(&terms.join(" + ")).await?;
                if type_of(&sum) != "decimal" {
                    return error(format!("{what} could not be computed: the adapter answered a `{}`", type_of(&sum)));
                }
                total = Some(sum);
            }
            return Ok(total.map_or(Total::Empty, Total::Decimal));
        }
        if only(&["double"]) {
            if numbers.len() > MAX_TERMS {
                return error(format!(
                    "{what} of more than {MAX_TERMS} double values is not supported: netcoredbg cannot evaluate an \
                     expression that long, and their printed values are rounded"
                ));
            }
            // As Enumerable.Sum does: from 0.0, in order.
            let terms: Vec<&str> = numbers.iter().map(|(k, _)| k.expr.as_str()).collect();
            let expression = format!("0.0 + {}", terms.join(" + "));
            let var = self.plain(&expression).await?;
            return Ok(Total::Double { var, expression });
        }
        let mut names: Vec<&str> = types.into_iter().collect();
        names.sort_unstable();
        error(format!("{what} of `{}` values is not supported", names.join("`, `")))
    }

    /// `Min()` / `Max()`: the first smallest or largest element, nulls ignored.
    async fn extreme(&mut self, mut seq: Seq, max: bool) -> Outcome<Answer> {
        let mut best: Option<(Keyed, usize)> = None;
        for i in 0..seq.items.len() {
            let candidate = self.keyed(&mut seq.items[i]).await?;
            if candidate.key == Key::Null {
                continue;
            }
            let better = match &best {
                None => true,
                Some((current, _)) => {
                    let order = self.compare(&candidate, current).await?;
                    order == if max { Ordering::Greater } else { Ordering::Less }
                }
            };
            if better {
                best = Some((candidate, i));
            }
        }
        match best {
            Some((_, i)) => {
                let item = seq.items.swap_remove(i);
                self.element(item, None).await
            }
            None => error("Sequence contains no elements"),
        }
    }

    /// The order of two values, as `Comparer<T>.Default` has it.
    async fn compare(&mut self, a: &Keyed, b: &Keyed) -> Outcome<Ordering> {
        match (&a.key, &b.key) {
            (Key::Null, Key::Null) => return Ok(Ordering::Equal),
            (Key::Null, _) => return Ok(Ordering::Less),
            (_, Key::Null) => return Ok(Ordering::Greater),
            (Key::Bool(x), Key::Bool(y)) => return Ok(x.cmp(y)),
            (Key::Char(x), Key::Char(y)) => return Ok(x.cmp(y)),
            (Key::Int(x), Key::Int(y)) => return Ok(x.cmp(y)),
            (Key::Float(x), Key::Float(y)) => {
                if x != y {
                    return Ok(if x < y { Ordering::Less } else { Ordering::Greater });
                }
                if a.expr == b.expr {
                    return Ok(Ordering::Equal);
                }
                // The same printed digits: only the adapter knows the full values.
                for (op, order) in [("<", Ordering::Less), (">", Ordering::Greater)] {
                    let v = self.plain(&format!("{} {op} {}", a.expr, b.expr)).await?;
                    if as_bool(&v).map_err(Fail::Error)? {
                        return Ok(order);
                    }
                }
                return Ok(Ordering::Equal);
            }
            _ => {}
        }
        if let (Some(x), Some(y)) = (a.key.dec(), b.key.dec()) {
            return Ok(x.cmp(&y));
        }
        // Strings (culture order), dates and anything else comparable.
        let v = self.native(&format!("{}.CompareTo({})", a.expr, b.expr)).await?;
        match v.as_ref().map(key_of) {
            Ok(Key::Int(n)) => Ok(n.cmp(&0)),
            _ => error(format!("`{}` values cannot be ordered by the debugger", a.type_name)),
        }
    }

    /// Equality, as `EqualityComparer<T>.Default` has it.
    async fn same(&mut self, a: &Keyed, b: &Keyed) -> Outcome<bool> {
        if a.key == Key::Null || b.key == Key::Null {
            return Ok(a.key == b.key);
        }
        if let (Some(x), Some(y)) = (a.key.canonical(), b.key.canonical()) {
            return Ok(x == y);
        }
        let text = match (&a.key, &b.key) {
            (Key::Float(x), Key::Float(y)) if x != y => return Ok(false),
            (Key::Float(_), Key::Float(_)) if a.expr == b.expr => return Ok(true),
            (x, y) if x.numeric() && y.numeric() => format!("{} == {}", a.expr, b.expr),
            _ => format!("{}.Equals({})", a.expr, b.expr),
        };
        let v = self.plain(&text).await?;
        as_bool(&v).map_err(|_| Fail::Error(format!("`{}` values cannot be compared by the debugger", a.type_name)))
    }

    async fn distinct(&mut self, items: Vec<Item>) -> Outcome<Vec<Item>> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut inexact: Vec<Keyed> = Vec::new();
        let mut kept = Vec::new();
        for mut item in items {
            let keyed = self.keyed(&mut item).await?;
            let new = match keyed.key.canonical() {
                Some(canonical) => seen.insert(canonical),
                None => {
                    let mut duplicate = false;
                    for other in &inexact {
                        if self.same(&keyed, other).await? {
                            duplicate = true;
                            break;
                        }
                    }
                    if !duplicate {
                        inexact.push(keyed);
                    }
                    !duplicate
                }
            };
            if new {
                kept.push(item);
            }
        }
        Ok(kept)
    }

    /// A stable merge sort: the comparison may need the adapter.
    async fn sort(&mut self, keys: &[Keyed], descending: bool) -> Outcome<Vec<usize>> {
        let mut order: Vec<usize> = (0..keys.len()).collect();
        let mut merged = Vec::with_capacity(order.len());
        let mut width = 1;
        while width < order.len() {
            merged.clear();
            for run in order.chunks(2 * width) {
                let (left, right) = run.split_at(width.min(run.len()));
                let (mut l, mut r) = (0, 0);
                while l < left.len() && r < right.len() {
                    let mut order = self.compare(&keys[right[r]], &keys[left[l]]).await?;
                    if descending {
                        order = order.reverse();
                    }
                    if order == Ordering::Less {
                        merged.push(right[r]);
                        r += 1;
                    } else {
                        merged.push(left[l]);
                        l += 1;
                    }
                }
                merged.extend_from_slice(&left[l..]);
                merged.extend_from_slice(&right[r..]);
            }
            std::mem::swap(&mut order, &mut merged);
            width *= 2;
        }
        Ok(order)
    }

    /// A sequence as the result: a value whose children are the elements.
    async fn sequence(&mut self, mut seq: Seq) -> Outcome<Answer> {
        let mut children = Vec::with_capacity(seq.items.len());
        for (i, item) in seq.items.iter_mut().enumerate() {
            let mut v = self.var_of(item, seq.pair.as_deref()).await?;
            v.name = format!("[{i}]");
            v.evaluate_name = match &item.elem {
                Elem::One(e) => Some(e.clone()),
                Elem::Pair { .. } => None,
            };
            children.push(v);
        }
        let n = children.len();
        let mut types: Vec<&str> = children.iter().filter(|c| !is_null(c)).map(type_of).collect();
        types.dedup();
        let element = match types.as_slice() {
            [one] => Some((*one).to_owned()),
            _ => seq.element.clone().or(seq.pair.clone()),
        };
        let (text, type_name) = match (seq.shape, element) {
            (Shape::Array, Some(t)) => (format!("{{{t}[{n}]}}"), format!("{t}[]")),
            (Shape::Array, None) => (format!("Count = {n}"), "System.Array".to_owned()),
            (Shape::List, Some(t)) => (format!("Count = {n}"), format!("System.Collections.Generic.List<{t}>")),
            (Shape::List, None) => (format!("Count = {n}"), "System.Collections.IList".to_owned()),
            (Shape::Enumerable, Some(t)) => {
                (format!("Count = {n}"), format!("System.Collections.Generic.IEnumerable<{t}>"))
            }
            (Shape::Enumerable, None) => (format!("Count = {n}"), "System.Collections.IEnumerable".to_owned()),
        };
        let reference = if n == 0 { 0 } else { self.host.keep(children) };
        Ok(Answer { var: Variable { variables_reference: reference, ..value(text, &type_name) }, splice: None })
    }

    // --- `Length` ------------------------------------------------------------------------

    /// netcoredbg has no `Length`: a string's is `get_Length()`, an array's is in its value.
    async fn lengths(&mut self, src: &str, tokens: &[Token]) -> Outcome<Option<Variable>> {
        let mut out = String::new();
        let mut pos = 0;
        for i in 1..tokens.len() {
            let called = tokens.get(i + 1).is_some_and(|t| t.is(src, "("));
            if !tokens[i].word(src, "Length") || !tokens[i - 1].is(src, ".") || called {
                continue;
            }
            let Some(first) = postfix_start(src, tokens, i - 1).filter(|f| tokens[*f].start >= pos) else { continue };
            let Ok(receiver) = self.native(slice(src, tokens, first, i - 1)).await? else { continue };
            if type_of(&receiver) == "string" && !is_null(&receiver) {
                out.push_str(&src[pos..tokens[i].start]);
                out.push_str("get_Length()");
            } else if let Some(n) = array_len(type_of(&receiver), &receiver.value) {
                out.push_str(&src[pos..tokens[first].start]);
                out.push_str(&n.to_string());
            } else {
                continue;
            }
            pos = tokens[i].end;
        }
        if pos == 0 {
            return Ok(None);
        }
        out.push_str(&src[pos..]);
        self.eval(out).await.map(Some)
    }

    // --- Type names ----------------------------------------------------------------------

    /// netcoredbg only knows fully qualified type names: `string.` is `System.String.`,
    /// and `Math.` is looked up in the namespaces the frame's source imports.
    async fn qualified(&mut self, src: &str, tokens: &[Token]) -> Outcome<Option<Variable>> {
        let mut roots: Vec<(usize, &str)> = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            let qualifier = tokens.get(i + 1).is_some_and(|n| n.is(src, "."));
            let member = i.checked_sub(1).is_some_and(|p| [".", "?.", "::", "->"].iter().any(|d| tokens[p].is(src, d)));
            if t.kind == Kind::Ident && qualifier && !member {
                roots.push((i, t.text(src)));
            }
        }
        let mut fixed: HashMap<&str, String> = HashMap::new();
        let mut unknown: Vec<&str> = Vec::new();
        for &(_, name) in &roots {
            if fixed.contains_key(name) || unknown.contains(&name) {
                continue;
            }
            if let Some((_, full)) = ALIASES.iter().find(|(alias, _)| *alias == name) {
                fixed.insert(name, (*full).to_owned());
                continue;
            }
            if !name.starts_with(char::is_uppercase) {
                continue;
            }
            let known = match self.types.get(name) {
                Some(known) => known.clone(),
                None => {
                    // A local, field or property of that name is not a type.
                    if self.native(name).await?.is_ok() {
                        self.types.insert(name.to_owned(), None);
                        continue;
                    }
                    unknown.push(name);
                    continue;
                }
            };
            if let Some(full) = known {
                fixed.insert(name, full);
            }
        }
        if fixed.is_empty() && unknown.is_empty() {
            return Ok(None);
        }
        let rewrite = |chosen: &HashMap<&str, String>| {
            let mut out = String::with_capacity(src.len() + 16);
            let mut pos = 0;
            for &(i, name) in &roots {
                if let Some(full) = chosen.get(name) {
                    out.push_str(&src[pos..tokens[i].start]);
                    out.push_str(full);
                    pos = tokens[i].end;
                }
            }
            out.push_str(&src[pos..]);
            out
        };
        if unknown.is_empty() {
            return Ok(self.native(&rewrite(&fixed)).await?.ok());
        }
        if self.namespaces.is_none() {
            self.namespaces = Some(self.host.namespaces().await);
        }
        let namespaces = self.namespaces.clone().unwrap_or_default();
        // The parts of a namespace are not types to look up in turn.
        unknown.retain(|name| !namespaces.iter().any(|ns| ns.split('.').next() == Some(*name)) && *name != "System");
        if unknown.is_empty() || namespaces.is_empty() {
            return Ok(None);
        }
        let combinations = namespaces.len().checked_pow(unknown.len() as u32).filter(|c| *c <= MAX_QUALIFICATIONS);
        let Some(combinations) = combinations else { return Ok(None) };
        for combination in 0..combinations {
            let mut chosen = fixed.clone();
            let mut rest = combination;
            for &name in &unknown {
                chosen.insert(name, format!("{}.{name}", namespaces[rest % namespaces.len()]));
                rest /= namespaces.len();
            }
            if let Ok(v) = self.native(&rewrite(&chosen)).await? {
                for &name in &unknown {
                    self.types.insert(name.to_owned(), chosen.get(name).cloned());
                }
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
}

enum Step {
    Seq(Seq),
    Value(Answer),
}

enum Total {
    Empty,
    Int { value: i128, long: bool },
    Decimal(Variable),
    Double { var: Variable, expression: String },
}

fn at_element(fail: Fail, index: usize) -> Fail {
    match fail {
        Fail::Error(message) | Fail::Skip(message) => Fail::Error(format!("element [{index}]: {message}")),
        fatal => fatal,
    }
}

/// `x == null`, `x != null`, `null == x`, `x is null`, `x is not null` in `lo..hi`:
/// the token range of `x` and whether the test is negated.
fn null_test(src: &str, tokens: &[Token], lo: usize, hi: usize) -> Option<(usize, usize, bool)> {
    let null = |i: usize| tokens[i].word(src, "null");
    let equality = |i: usize| match tokens[i].text(src) {
        "==" if tokens[i].kind == Kind::Punct => Some(false),
        "!=" if tokens[i].kind == Kind::Punct => Some(true),
        _ => None,
    };
    let n = hi.checked_sub(lo)?;
    let found = if n >= 3 && null(hi - 1) && equality(hi - 2).is_some() {
        (lo, hi - 2, equality(hi - 2)?)
    } else if n >= 3 && null(lo) && equality(lo + 1).is_some() {
        (lo + 2, hi, equality(lo + 1)?)
    } else if n >= 4 && null(hi - 1) && tokens[hi - 2].word(src, "not") && tokens[hi - 3].word(src, "is") {
        (lo, hi - 3, true)
    } else if n >= 3 && null(hi - 1) && tokens[hi - 2].word(src, "is") {
        (lo, hi - 2, false)
    } else {
        return None;
    };
    is_postfix(src, tokens, found.0, found.1).then_some(found)
}

/// Namespaces of `using` directives (and `namespace` declarations) in C# source.
pub(crate) fn source_namespaces(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let line = line.strip_prefix("global ").map(str::trim_start).unwrap_or(line);
        let name = if let Some(rest) = line.strip_prefix("using ") {
            match rest.trim().strip_suffix(';') {
                Some(name) if !name.contains(['=', '(', ' ']) => name,
                _ => continue,
            }
        } else if let Some(rest) = line.strip_prefix("namespace ") {
            rest.trim().trim_end_matches([';', '{']).trim_end()
        } else {
            continue;
        };
        let name = name.strip_prefix("global::").unwrap_or(name);
        let valid = !name.is_empty()
            && name.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_'));
        if valid && !found.iter().any(|f| f == name) {
            found.push(name.to_owned());
        }
    }
    found
}

/// Namespaces enclosing a frame named like `App.Services.Orders.Total(int)`, nearest first.
pub(crate) fn frame_namespaces(frame: &str) -> Vec<String> {
    let path = frame.split('(').next().unwrap_or_default();
    let mut parts: Vec<&str> = path.split('.').collect();
    parts.pop();
    let simple = |p: &&str| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_');
    let parts: Vec<&str> = parts.into_iter().take_while(simple).collect();
    (1..=parts.len()).rev().map(|n| parts[..n].join(".")).collect()
}

/// A debug session as the evaluator's [`Host`].
pub(crate) struct SessionHost<'a> {
    session: &'a Session,
    frame_id: Option<i64>,
    context: &'a str,
    epoch: u64,
}

impl<'a> SessionHost<'a> {
    pub fn new(session: &'a Session, frame_id: Option<i64>, context: &'a str) -> Self {
        Self { session, frame_id, context, epoch: session.synthetic.lock().epoch() }
    }
}

impl Host for SessionHost<'_> {
    fn evaluate<'a>(&'a self, expression: &'a str) -> BoxFuture<'a, Result<Variable, DapError>> {
        Box::pin(self.session.evaluate(self.frame_id, expression, self.context))
    }

    fn variables(&self, reference: i64, count: Option<usize>) -> BoxFuture<'_, Result<Vec<Variable>, DapError>> {
        Box::pin(self.session.variables(reference, count))
    }

    fn keep(&self, children: Vec<Variable>) -> i64 {
        self.session.synthetic.lock().insert(children)
    }

    fn cancelled(&self) -> bool {
        self.session.terminated.load(AtomicOrdering::Acquire) || self.session.synthetic.lock().epoch() != self.epoch
    }

    fn namespaces(&self) -> BoxFuture<'_, Vec<String>> {
        let frame = self.frame_id.and_then(|f| self.session.frames.lock().get(&f).cloned());
        Box::pin(async move {
            let Some((name, path)) = frame else { return Vec::new() };
            tokio::task::spawn_blocking(move || namespaces_of(&name, path.as_deref())).await.unwrap_or_default()
        })
    }
}

/// Where C# looks up an unqualified type name used in a frame: the namespaces around
/// the frame's method, then the source file's `using`s, then the project's global ones
/// (the SDK writes the implicit usings to `obj/**/*.GlobalUsings.g.cs`).
fn namespaces_of(frame: &str, source: Option<&Path>) -> Vec<String> {
    let mut found = frame_namespaces(frame);
    if let Some(source) = source {
        found.extend(std::fs::read_to_string(source).map(|text| source_namespaces(&text)).unwrap_or_default());
        let is_project = |dir: &&Path| {
            std::fs::read_dir(dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    e.path().extension().is_some_and(|x| ["csproj", "fsproj", "vbproj"].iter().any(|p| x == *p))
                })
            })
        };
        if let Some(project) = source.ancestors().skip(1).find(is_project) {
            let mut generated = Vec::new();
            global_usings(&project.join("obj"), 3, &mut generated);
            generated.sort();
            for file in generated {
                found.extend(std::fs::read_to_string(file).map(|text| source_namespaces(&text)).unwrap_or_default());
            }
        }
    }
    let mut seen = HashSet::new();
    found.retain(|n| seen.insert(n.clone()));
    found
}

fn global_usings(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                global_usings(&path, depth - 1, found);
            }
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".GlobalUsings.g.cs")) {
            found.push(path);
        }
    }
}

#[cfg(test)]
#[path = "dotnet_eval_tests.rs"]
pub(crate) mod tests;
