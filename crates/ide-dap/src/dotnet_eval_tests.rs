//! The netcoredbg evaluation helper against a scripted host that behaves like netcoredbg
//! for the expressions used here (values seen with netcoredbg 3.2.0).

use super::*;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrd};

fn var(value: &str, ty: &str, reference: i64, path: &str) -> Variable {
    Variable {
        name: path.to_owned(),
        value: value.to_owned(),
        type_name: Some(ty.to_owned()),
        variables_reference: reference,
        evaluate_name: Some(path.to_owned()),
    }
}

fn child(name: &str, value: &str, ty: &str, reference: i64, path: &str) -> Variable {
    Variable { name: name.to_owned(), ..var(value, ty, reference, path) }
}

#[derive(Default)]
struct Mock {
    values: HashMap<String, Variable>,
    children: HashMap<i64, Vec<Variable>>,
    asked: Mutex<Vec<String>>,
    kept: Mutex<Vec<Vec<Variable>>>,
    cancelled: AtomicBool,
    namespaces: Vec<String>,
}

impl Mock {
    fn value(&mut self, path: &str, value: &str, ty: &str) {
        self.values.insert(path.into(), var(value, ty, 0, path));
    }

    /// A `List<int>` (or other element type) with spare capacity, laid out like the runtime's.
    fn list(&mut self, name: &str, reference: i64, ty: &str, elements: &[&str]) {
        let items = reference + 1;
        let capacity = elements.len().next_power_of_two().max(4);
        self.values.insert(
            name.into(),
            var(
                &format!("{{System.Collections.Generic.List<{ty}>}}"),
                &format!("System.Collections.Generic.List<{ty}>"),
                reference,
                name,
            ),
        );
        self.children.insert(
            reference,
            vec![
                child("_items", &format!("{{{ty}[{capacity}]}}"), &format!("{ty}[]"), items, &format!("{name}._items")),
                child("_size", &elements.len().to_string(), "int", 0, &format!("{name}._size")),
                child("Count", &elements.len().to_string(), "int", 0, &format!("{name}.Count")),
            ],
        );
        let mut slots = Vec::new();
        for i in 0..capacity {
            let path = format!("{name}._items[{i}]");
            let value = elements.get(i).copied().unwrap_or(if ty == "int" { "0" } else { "null" });
            slots.push(child(&format!("[{i}]"), value, ty, 0, &path));
            self.value(&path, value, ty);
        }
        self.children.insert(items, slots);
    }

    fn array(&mut self, name: &str, reference: i64, ty: &str, elements: &[&str]) {
        self.values
            .insert(name.into(), var(&format!("{{{ty}[{}]}}", elements.len()), &format!("{ty}[]"), reference, name));
        let mut slots = Vec::new();
        for (i, value) in elements.iter().enumerate() {
            let path = format!("{name}[{i}]");
            slots.push(child(&format!("[{i}]"), value, ty, 0, &path));
            self.value(&path, value, ty);
        }
        self.children.insert(reference, slots);
    }

    /// A `Dictionary<string, int>` with one free slot (`next == -2`).
    fn dictionary(&mut self, name: &str, reference: i64) {
        self.values.insert(
            name.into(),
            var(
                "{System.Collections.Generic.Dictionary<string, int>}",
                "System.Collections.Generic.Dictionary<string, int>",
                reference,
                name,
            ),
        );
        let entries = reference + 1;
        self.children.insert(
            reference,
            vec![
                child("_buckets", "{int[3]}", "int[]", 0, &format!("{name}._buckets")),
                child(
                    "_entries",
                    "{System.Collections.Generic.Dictionary<string, int>.Entry[3]}",
                    "System.Collections.Generic.Dictionary<string, int>.Entry[]",
                    entries,
                    &format!("{name}._entries"),
                ),
                child("_count", "3", "int", 0, &format!("{name}._count")),
            ],
        );
        let mut slots = Vec::new();
        for (i, (key, value, next)) in
            [("\"a\"", "1", "-1"), ("null", "2", "-2"), ("\"c\"", "3", "-1")].iter().enumerate()
        {
            let path = format!("{name}._entries[{i}]");
            let slot = entries + 10 + i as i64;
            slots.push(child(&format!("[{i}]"), "{Entry}", "Entry", slot, &path));
            self.children.insert(
                slot,
                vec![
                    child("hashCode", "1", "uint", 0, &format!("{path}.hashCode")),
                    child("next", next, "int", 0, &format!("{path}.next")),
                    child("key", key, "string", 0, &format!("{path}.key")),
                    child("value", value, "int", 0, &format!("{path}.value")),
                ],
            );
            self.value(&format!("{path}.key"), key, "string");
            self.value(&format!("{path}.value"), value, "int");
        }
        self.children.insert(entries, slots);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().clone()
    }

    /// A tiny evaluator: known paths, and `a OP b` over integers and decimals.
    fn answer(&self, expression: &str) -> Result<Variable, String> {
        let e = expression.trim();
        if e.contains("=>") {
            return Err("NetCoreDbg.Evaluation+SyntaxKindNotImplementedException: Parameter not implemented!".into());
        }
        if let Some(v) = self.values.get(e) {
            return Ok(v.clone());
        }
        if let Some(inner) = e.strip_prefix('(').and_then(|i| i.strip_suffix(')'))
            && let Ok(v) = self.answer(inner)
        {
            return Ok(v);
        }
        if let Ok(n) = e.parse::<i64>() {
            return Ok(var(&n.to_string(), "int", 0, e));
        }
        if e.contains('.') && e.parse::<f64>().is_ok() {
            return Ok(var(e, "double", 0, e));
        }
        for op in [" + ", " / ", " >= ", " <= ", " > ", " < ", " == ", " != ", " % ", " * "] {
            if let Some((a, b)) = e.rsplit_once(op) {
                let num = |s: &str| -> Result<(f64, String), String> {
                    let v = self.answer(s)?;
                    Ok((
                        v.value.trim_end_matches('m').parse::<f64>().map_err(|_| format!("not a number: {s}"))?,
                        type_of(&v).to_owned(),
                    ))
                };
                let ((x, tx), (y, ty)) = (num(a)?, num(b)?);
                let ty = if tx == "decimal" || ty == "decimal" { "decimal" } else { "int" };
                let n = |r: f64| Ok(var(&format!("{r}"), ty, 0, e));
                return match op.trim() {
                    "+" => n(x + y),
                    "*" => n(x * y),
                    "/" => n(x / y),
                    "%" => n(x % y),
                    ">" => Ok(boolean(x > y)),
                    "<" => Ok(boolean(x < y)),
                    ">=" => Ok(boolean(x >= y)),
                    "<=" => Ok(boolean(x <= y)),
                    "==" => Ok(boolean(x == y)),
                    _ => Ok(boolean(x != y)),
                };
            }
        }
        if let Some(rest) = e.strip_suffix("m")
            && rest.parse::<f64>().is_ok()
        {
            return Ok(var(rest, "decimal", 0, e));
        }
        Err(format!("error: The name '{e}' does not exist in the current context"))
    }
}

impl Host for Mock {
    fn evaluate<'a>(&'a self, expression: &'a str) -> BoxFuture<'a, Result<Variable, DapError>> {
        self.asked.lock().push(expression.to_owned());
        let answer = self.answer(expression);
        Box::pin(async move { answer.map_err(|message| DapError::Failed { command: "evaluate".into(), message }) })
    }

    fn variables(&self, reference: i64, count: Option<usize>) -> BoxFuture<'_, Result<Vec<Variable>, DapError>> {
        let mut children = self
            .children
            .get(&reference)
            .cloned()
            .or_else(|| {
                Synthetic::is(reference)
                    .then(|| self.kept.lock().get((reference - SYNTHETIC_BASE) as usize).cloned())
                    .flatten()
            })
            .unwrap_or_default();
        children.truncate(count.unwrap_or(usize::MAX));
        Box::pin(async move { Ok(children) })
    }

    fn keep(&self, children: Vec<Variable>) -> i64 {
        let mut kept = self.kept.lock();
        kept.push(children);
        SYNTHETIC_BASE + kept.len() as i64 - 1
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrd::Acquire)
    }

    fn namespaces(&self) -> BoxFuture<'_, Vec<String>> {
        let found = self.namespaces.clone();
        Box::pin(async move { found })
    }
}

fn demo() -> Mock {
    let mut m = Mock::default();
    m.list("list", 10, "int", &["1", "2", "3", "4", "5"]);
    m.list("empty", 20, "int", &[]);
    m.list("totals", 30, "decimal", &["12.5", "3", "40"]);
    m.array("numbers", 40, "int", &["5", "3", "8", "3"]);
    m.dictionary("dict", 50);
    for (name, reference) in [("Keys", 70), ("Values", 71)] {
        let ty = format!("System.Collections.Generic.Dictionary<string, int>.{}Collection", &name[..name.len() - 1]);
        let path = format!("dict.{name}");
        m.values.insert(path.clone(), var("{collection}", &ty, reference, &path));
        let dict = m.values["dict"].clone();
        m.children.insert(
            reference,
            vec![Variable { name: "_dictionary".into(), evaluate_name: Some(format!("{path}._dictionary")), ..dict }],
        );
    }
    m.value("s", "\"hello\"", "string");
    m.value("none", "null", "Order");
    m
}

async fn eval(m: &Mock, expression: &str) -> IdeResult<Variable> {
    let native = m.evaluate(expression).await;
    assist(m, expression, native).await
}

async fn value_of(m: &Mock, expression: &str) -> String {
    match eval(m, expression).await {
        Ok(v) => v.value,
        Err(e) => panic!("{expression}: {}", e.message),
    }
}

async fn error_of(m: &Mock, expression: &str) -> String {
    match eval(m, expression).await {
        Ok(v) => panic!("{expression} evaluated to {}", v.value),
        Err(e) => e.message,
    }
}

#[test]
fn tokenizer_keeps_literals_whole() {
    let src = r#"x.Name == "a => b, (c)" && c == ')' && @"q""x" != """raw "" text""""#;
    let tokens = tokenize(src).unwrap();
    let texts: Vec<&str> = tokens.iter().filter(|t| t.kind == Kind::Text).map(|t| t.text(src)).collect();
    assert_eq!(texts, [r#""a => b, (c)""#, "')'", r#"@"q""x""#, r#""""raw "" text""""#]);
    assert!(tokenize("$\"{x}\"").is_err());
    assert!(tokenize("a // b").is_err());
    let num = tokenize("1.5e-3m + 0x1F").unwrap();
    assert_eq!(num.iter().map(|t| t.text("1.5e-3m + 0x1F")).collect::<Vec<_>>(), ["1.5e-3m", "+", "0x1F"]);
}

#[test]
fn chains_are_found_with_their_receiver() {
    let src = "1 + orders[0].Lines.Where(l => l.Total > 2).Select(l => l.Id).Count() * 2";
    let tokens = tokenize(src).unwrap();
    let chain = find_chain(src, &tokens, 0).unwrap();
    assert_eq!(chain.receiver, "orders[0].Lines");
    assert_eq!(chain.calls.iter().map(|c| c.name).collect::<Vec<_>>(), ["Where", "Select", "Count"]);
    assert_eq!(&src[chain.start..chain.end], "orders[0].Lines.Where(l => l.Total > 2).Select(l => l.Id).Count()");
    assert_eq!(chain.calls[0].args, ["l => l.Total > 2"]);

    let src = "(a.b).Sum()";
    let tokens = tokenize(src).unwrap();
    assert_eq!(find_chain(src, &tokens, 0).unwrap().receiver, "(a.b)");
    let src = "Get().Count(x => x > 1, y)";
    let tokens = tokenize(src).unwrap();
    let chain = find_chain(src, &tokens, 0).unwrap();
    assert_eq!((chain.receiver, chain.calls[0].args.len()), ("Get()", 2));
    let src = "xs.Where<int>(x => x > 1).ToList()";
    let tokens = tokenize(src).unwrap();
    assert_eq!(find_chain(src, &tokens, 0).unwrap().calls.len(), 2);
    let src = "a.Max";
    assert!(find_chain(src, &tokenize(src).unwrap(), 0).is_none(), "a property, not a call");
}

#[test]
fn lambdas_parse_and_substitute_only_the_parameter() {
    let f = Func::parse("x => x.x > x.Count(x: 1) && \"x\" != x.Name && 'x' == x").unwrap();
    let applied = f.apply(&Elem::One("list._items[0]".into()), 0).unwrap();
    assert_eq!(
        applied,
        "list._items[0].x > list._items[0].Count(x: 1) && \"x\" != list._items[0].Name && 'x' == list._items[0]"
    );
    let f = Func::parse("(item, i) => item > i").unwrap();
    assert_eq!(f.apply(&Elem::One("a[3]".into()), 3).unwrap(), "a[3] > 3");
    let f = Func::parse("o => o.Total * 2").unwrap();
    assert_eq!(f.apply(&Elem::One("Get()[1]".into()), 1).unwrap(), "(Get()[1]).Total * 2");
    let pair = Elem::Pair { key: "d._entries[0].key".into(), value: "d._entries[0].value".into() };
    let f = Func::parse("kv => kv.Value > 1 && kv.Key != null").unwrap();
    assert_eq!(f.apply(&pair, 0).unwrap(), "d._entries[0].value > 1 && d._entries[0].key != null");
    assert!(Func::parse("kv => kv").unwrap().apply(&pair, 0).is_err());
    assert_eq!(Func::parse("Check").unwrap(), Func::Group("Check".into()));
    assert!(Func::parse("x => { return x; }").is_err());
    assert!(Func::parse("(int x) => x").is_err());
    assert!(Func::parse("x + 1").is_err());
}

#[test]
fn decimals_compare_exactly() {
    let d = |s: &str| Dec::parse(s).unwrap();
    assert!(d("12.5") > d("3"));
    assert!(d("-1.5") < d("-1.25"));
    assert_eq!(d("1.10"), d("1.1"));
    assert_eq!(d("-0"), d("0"));
    assert!(d("100") > d("99.999"));
    assert!(Dec::parse("1e5").is_none());
}

#[tokio::test]
async fn the_acceptance_expression_evaluates() {
    let m = demo();
    assert_eq!(value_of(&m, "list.Where(x => x > 2).Count()").await, "3");
    let asked = m.asked();
    assert!(asked.contains(&"list._items[0] > 2".to_owned()), "{asked:?}");
    assert!(!asked.iter().any(|a| a.contains("_items[5]")), "spare capacity is not read: {asked:?}");
}

#[tokio::test]
async fn each_operator_evaluates() {
    let m = demo();
    for (expression, expected) in [
        ("list.Count(x => x % 2 == 0)", "2"),
        ("list.Count()", "5"),
        ("list.Any(x => x > 4)", "true"),
        ("list.Any(x => x > 5)", "false"),
        ("empty.Any()", "false"),
        ("list.All(x => x > 0)", "true"),
        ("list.All(x => x > 1)", "false"),
        ("list.First(x => x > 2)", "3"),
        ("list.FirstOrDefault(x => x > 9)", "0"),
        ("list.Last()", "5"),
        ("list.LastOrDefault(x => x < 3)", "2"),
        ("list.Single(x => x == 4)", "4"),
        ("list.SingleOrDefault(x => x > 9)", "0"),
        ("list.ElementAt(1)", "2"),
        ("list.ElementAtOrDefault(9)", "0"),
        ("list.Skip(1).Take(2).Sum()", "5"),
        ("list.Sum()", "15"),
        ("list.Sum(x => x * 2)", "30"),
        ("list.Min()", "1"),
        ("list.Max(x => x % 3)", "2"),
        ("list.Contains(3)", "true"),
        ("list.Contains(9)", "false"),
        ("list.TakeWhile(x => x < 3).Count()", "2"),
        ("list.SkipWhile(x => x < 3).First()", "3"),
        ("numbers.Distinct().Count()", "3"),
        ("numbers.OrderBy(x => x).First()", "3"),
        ("numbers.OrderByDescending(x => x).First()", "8"),
        ("numbers.Where(x => x > 3).ToList().Count()", "2"),
        ("numbers.Max()", "8"),
        ("totals.Max()", "40"),
        ("totals.Sum()", "55.5"),
        ("empty.Sum()", "0"),
        ("dict.Count()", "2"),
        ("dict.Where(kv => kv.Value > 1).Count()", "1"),
        ("dict.Keys.Count()", "2"),
        ("dict.Values.Sum()", "4"),
        ("list.Where(x => x > 2).Count() + 1", "4"),
    ] {
        assert_eq!(value_of(&m, expression).await, expected, "{expression}");
    }
    assert_eq!(value_of(&m, "list.Average()").await, "3");
}

#[tokio::test]
async fn sequences_are_expandable_with_element_expressions() {
    let m = demo();
    let v = eval(&m, "list.Where(x => x > 2).Select(x => x * 10)").await.unwrap();
    assert_eq!(v.value, "Count = 3");
    assert!(Synthetic::is(v.variables_reference));
    let children = m.variables(v.variables_reference, None).await.unwrap();
    let shown: Vec<(&str, &str, Option<&str>)> =
        children.iter().map(|c| (c.name.as_str(), c.value.as_str(), c.evaluate_name.as_deref())).collect();
    assert_eq!(shown[0], ("[0]", "30", Some("list._items[2] * 10")));
    assert_eq!(shown.len(), 3);
    let v = eval(&m, "numbers.Where(x => x > 3).ToArray()").await.unwrap();
    assert_eq!((v.value.as_str(), v.type_name.as_deref()), ("{int[2]}", Some("int[]")));
    let v = eval(&m, "dict.Where(kv => kv.Value > 0)").await.unwrap();
    let entries = m.variables(v.variables_reference, None).await.unwrap();
    assert_eq!(entries.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(), ["[\"a\", 1]", "[\"c\", 3]"]);
    let parts = m.variables(entries[0].variables_reference, None).await.unwrap();
    assert_eq!(parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Key", "Value"]);
    assert_eq!(eval(&m, "empty.Where(x => x > 2)").await.unwrap().variables_reference, 0);
}

#[tokio::test]
async fn errors_are_reported_and_nothing_is_guessed() {
    let m = demo();
    assert_eq!(error_of(&m, "empty.First()").await, "Sequence contains no elements");
    assert_eq!(error_of(&m, "list.Single(x => x > 2)").await, "Sequence contains more than one matching element");
    assert!(error_of(&m, "list.ElementAt(9)").await.starts_with("Index was out of range"));
    assert!(error_of(&m, "list.Where(x => { return x > 1; }).Count()").await.contains("statement lambdas"));
    assert!(error_of(&m, "list.Where(x => x > 2).Reverse()").await.contains("`Reverse` cannot follow"));
    assert!(error_of(&m, "list.Where(x => x.Nope).Count()").await.starts_with("element [0]:"));
    assert!(error_of(&m, "none.Where(x => x > 1).Count()").await.contains("is null"));
    assert!(error_of(&m, "s.Where(c => c > 1).Count()").await.contains("not a collection"));
    // Not a chain over a collection: the adapter's own error.
    assert!(error_of(&m, "nosuch + 1").await.contains("does not exist"));
}

#[tokio::test]
async fn collections_over_the_cap_are_refused() {
    let mut m = demo();
    m.values.insert("huge".into(), var("{int[20000]}", "int[]", 99, "huge"));
    let message = error_of(&m, "huge.Count(x => x > 1)").await;
    assert!(message.contains(&MAX_ELEMENTS.to_string()), "{message}");
    assert!(m.asked().iter().all(|a| !a.starts_with("huge[")), "no element was read");
}

#[tokio::test]
async fn cancellation_stops_the_evaluation() {
    let m = demo();
    m.cancelled.store(true, AtomicOrd::Release);
    let err = eval(&m, "list.Where(x => x > 2).Count()").await.unwrap_err();
    assert_eq!(err.kind, ide_core::error::ErrorKind::Cancelled);
}

#[tokio::test]
async fn logic_and_null_tests_are_helped() {
    let mut m = demo();
    m.value("order", "{Order}", "Order");
    assert_eq!(value_of(&m, "none == null").await, "true");
    assert_eq!(value_of(&m, "order != null").await, "true");
    assert_eq!(value_of(&m, "order is not null").await, "true");
    // The right operand is not evaluated once the left decides.
    assert_eq!(value_of(&m, "none == null || none.Total > 2").await, "true");
    assert!(!m.asked().contains(&"none.Total > 2".to_owned()));
}

#[tokio::test]
async fn type_names_are_qualified() {
    let mut m = demo();
    m.namespaces = vec!["App".into(), "System".into()];
    m.values.insert("System.Math.Max(1, 2)".into(), var("2", "int", 0, ""));
    m.values.insert("System.String.IsNullOrEmpty(s)".into(), var("false", "bool", 0, ""));
    m.values.insert("s.get_Length()".into(), var("5", "int", 0, ""));
    assert_eq!(value_of(&m, "Math.Max(1, 2)").await, "2");
    assert_eq!(value_of(&m, "string.IsNullOrEmpty(s)").await, "false");
    assert_eq!(value_of(&m, "s.Length").await, "5");
    assert_eq!(value_of(&m, "numbers.Length").await, "4");
}

#[test]
fn namespaces_come_from_the_frame_and_the_source() {
    // The class itself comes first: its nested types are found too.
    assert_eq!(frame_namespaces("Shop.Orders.Basket.Run()"), ["Shop.Orders.Basket", "Shop.Orders", "Shop"]);
    assert_eq!(frame_namespaces("Program.Main()"), ["Program"]);
    let source = "global using global::System.Linq;\nusing System;\nusing Json = System.Text.Json;\nusing static System.Math;\nnamespace Shop.Api;\n";
    assert_eq!(source_namespaces(source), ["System.Linq", "System", "Shop.Api"]);
}

#[test]
fn synthetic_references_are_dropped_on_resume() {
    let mut store = Synthetic::default();
    let r = store.insert(vec![var("1", "int", 0, "a")]);
    assert!(Synthetic::is(r) && store.get(r).is_some());
    let epoch = store.epoch();
    store.clear();
    assert!(store.get(r).is_none());
    assert_ne!(store.epoch(), epoch);
}
