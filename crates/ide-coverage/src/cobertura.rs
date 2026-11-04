//! Cobertura XML reports (Microsoft Code Coverage, coverlet, ReportGenerator):
//!
//! ```xml
//! <coverage>
//!   <sources><source>/s/src/</source></sources>
//!   <packages><package name="App"><classes>
//!     <class name="App.Calc" filename="Calc.cs">
//!       <methods>…</methods>
//!       <lines>
//!         <line number="5" hits="3" branch="true" condition-coverage="50% (1/2)"/>
//! ```
//!
//! Only a class's own `<lines>` are read: its methods repeat the same lines.
//! `filename` is absolute or relative to one of the `<source>` directories.

use crate::Accumulator;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::path::{Path, PathBuf};

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).map(|a| a.value.into_owned())
}

/// `50% (1/2)` → `(1, 2)`.
fn condition_coverage(text: &str) -> Option<(u32, u32)> {
    let inner = text.split('(').nth(1)?.split(')').next()?;
    let (covered, total) = inner.split_once('/')?;
    Some((covered.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// The file a class's `filename` names: absolute, else the first source directory that has it.
fn resolve(filename: &str, sources: &[PathBuf], base: &Path) -> PathBuf {
    let name = filename.replace('\\', "/");
    let p = Path::new(&name);
    if p.is_absolute() || name.as_bytes().get(1) == Some(&b':') {
        return p.to_path_buf();
    }
    sources
        .iter()
        .map(|s| s.join(p))
        .find(|c| c.is_file())
        .unwrap_or_else(|| sources.first().map_or_else(|| base.join(p), |s| s.join(p)))
}

/// Parses one report into `acc`; relative names without a matching source resolve against `base`.
pub(crate) fn parse_into(xml: &str, base: &Path, acc: &mut Accumulator) -> Result<(), quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut in_source = false;
    let mut file: Option<PathBuf> = None;
    let mut in_method = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                "source" => in_source = true,
                "class" => {
                    let name = attr(&e, "filename").unwrap_or_default();
                    file = (!name.is_empty()).then(|| resolve(&name, &sources, base));
                    if let Some(f) = &file {
                        acc.file(f);
                    }
                }
                "method" => in_method = true,
                "line" if !in_method => {
                    let Some(f) = file.as_deref() else { continue };
                    let number: u32 = attr(&e, "number").and_then(|v| v.parse().ok()).unwrap_or(0);
                    if number == 0 {
                        continue;
                    }
                    let hits: u64 = attr(&e, "hits").and_then(|v| v.parse().ok()).unwrap_or(0);
                    acc.line(f, number - 1, hits);
                    if attr(&e, "branch").is_some_and(|b| b.eq_ignore_ascii_case("true"))
                        && let Some((covered, total)) =
                            attr(&e, "condition-coverage").as_deref().and_then(condition_coverage)
                    {
                        acc.has_branches = true;
                        for i in 0..total {
                            acc.branch(f, number - 1, i.to_string(), i < covered);
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_source => {
                let dir = t.into_inner().trim().to_owned();
                if !dir.is_empty() {
                    sources.push(PathBuf::from(dir.replace('\\', "/")));
                }
            }
            Event::End(e) => match e.name().as_ref() {
                "source" => in_source = false,
                "method" => in_method = false,
                "class" => file = None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<coverage line-rate="0.5" branch-rate="0.5" version="1.9">
  <sources><source>/s/src/</source></sources>
  <packages>
    <package name="Fixture.Core">
      <classes>
        <class name="Fixture.Core.Greeter" filename="Fixture.Core/Greeter.cs">
          <methods>
            <method name="Greet" signature="(string)">
              <lines><line number="13" hits="4" branch="false" /></lines>
            </method>
          </methods>
          <lines>
            <line number="13" hits="4" branch="false" />
            <line number="15" hits="0" branch="true" condition-coverage="50% (1/2)" />
            <line number="17" hits="4" branch="false" />
          </lines>
        </class>
        <class name="Other" filename="/abs/Other.cs">
          <lines><line number="1" hits="0" /></lines>
        </class>
      </classes>
    </package>
  </packages>
</coverage>"#;

    #[test]
    fn reads_class_lines_once_with_branches() {
        let mut acc = Accumulator::default();
        parse_into(REPORT, Path::new("/base"), &mut acc).unwrap();
        let suite = acc.finish("Microsoft Code Coverage");
        assert!(suite.has_branches);
        assert_eq!(suite.files.len(), 2);
        let greeter = suite
            .files
            .iter()
            .find(|f| f.path.as_ref() as &Path == Path::new("/s/src/Fixture.Core/Greeter.cs"))
            .unwrap();
        assert_eq!((greeter.lines_total, greeter.lines_covered), (3, 2));
        assert_eq!(greeter.lines[0].hits, 4, "method lines are not counted twice");
        assert_eq!((greeter.lines[1].line, greeter.branches_total, greeter.branches_covered), (14, 2, 1));
        assert!(suite.files.iter().any(|f| f.path.as_ref() as &Path == Path::new("/abs/Other.cs")));
    }

    #[test]
    fn reads_condition_coverage() {
        assert_eq!(condition_coverage("50% (1/2)"), Some((1, 2)));
        assert_eq!(condition_coverage("100%"), None);
    }
}
