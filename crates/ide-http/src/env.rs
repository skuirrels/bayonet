//! Environments (`http-client.env.json`, `http-client.private.env.json`) and
//! `{{variable}}` substitution.
//!
//! ```json
//! { "$shared": { "version": "v1" }, "dev": { "host": "localhost:8080" } }
//! ```
//!
//! Lookup order for `{{name}}`: file variables (`@name = …`), the private
//! environment, the public environment, private `$shared`, public `$shared`.
//! Dynamic variables: `$uuid` / `$random.uuid`, `$timestamp`, `$isoTimestamp`,
//! `$randomInt` / `$random.integer`, and Visual Studio's (PLT-3): `$guid`,
//! `$randomInt min max`, `$timestamp [offset unit]`, `$datetime` / `$localDatetime
//! rfc1123|iso8601|"custom" [offset unit]`, `$processEnv NAME` and `$dotenv NAME`.
//! Visual Studio's user file `http-client.env.json.user` counts as private.

use ide_core::{IdeError, IdeResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub const PUBLIC_FILE: &str = "http-client.env.json";
pub const PRIVATE_FILE: &str = "http-client.private.env.json";
/// Visual Studio's per-user environment file.
pub const USER_FILE: &str = "http-client.env.json.user";
const SHARED: &str = "$shared";

type EnvMap = BTreeMap<String, BTreeMap<String, String>>;

/// The environment files that apply to a request file.
#[derive(Debug, Default, Clone)]
pub struct Environments {
    public: EnvMap,
    private: EnvMap,
}

/// Folders searched for env files: the request file's folder and its
/// ancestors up to (and including) `root`; `root` itself when the file is outside it.
fn search_dirs(file: Option<&Path>, root: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(mut dir) = file.and_then(Path::parent) {
        loop {
            dirs.push(dir.to_path_buf());
            if root.is_none_or(|r| dir == r || !dir.starts_with(r)) {
                break;
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
    }
    if let Some(r) = root
        && !dirs.iter().any(|d| d == r)
    {
        dirs.push(r.to_path_buf());
    }
    dirs
}

fn parse_env(text: &str) -> IdeResult<EnvMap> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| IdeError::invalid(format!("invalid environment file: {e}")))?;
    let mut out = EnvMap::new();
    let Some(envs) = value.as_object() else { return Ok(out) };
    for (env, vars) in envs {
        let Some(vars) = vars.as_object() else { continue };
        let mut map = BTreeMap::new();
        for (k, v) in vars {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                // Objects configure SSL / auth in IDEA; not variables.
                _ => continue,
            };
            map.insert(k.clone(), s);
        }
        out.insert(env.clone(), map);
    }
    Ok(out)
}

fn read_first(dirs: &[PathBuf], name: &str) -> IdeResult<EnvMap> {
    for d in dirs {
        let p = d.join(name);
        if let Ok(text) = std::fs::read_to_string(&p) {
            return parse_env(&text).map_err(|e| IdeError::invalid(format!("{}: {}", p.display(), e.message)));
        }
    }
    Ok(EnvMap::new())
}

impl Environments {
    pub fn load(file: Option<&Path>, root: Option<&Path>) -> IdeResult<Self> {
        let dirs = search_dirs(file, root);
        let mut private = read_first(&dirs, PRIVATE_FILE)?;
        for (env, vars) in read_first(&dirs, USER_FILE)? {
            let into = private.entry(env).or_default();
            for (k, v) in vars {
                into.entry(k).or_insert(v);
            }
        }
        Ok(Self { public: read_first(&dirs, PUBLIC_FILE)?, private })
    }

    pub fn from_json(public: &str, private: &str) -> IdeResult<Self> {
        let p = |s: &str| if s.trim().is_empty() { Ok(EnvMap::new()) } else { parse_env(s) };
        Ok(Self { public: p(public)?, private: p(private)? })
    }

    /// Environment names (excluding `$shared`), sorted.
    pub fn names(&self) -> Vec<String> {
        let set: BTreeSet<&String> = self.public.keys().chain(self.private.keys()).filter(|k| *k != SHARED).collect();
        set.into_iter().cloned().collect()
    }

    /// Variables of `env` merged in lookup order (first wins).
    pub fn variables(&self, env: Option<&str>) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut layer = |m: Option<&BTreeMap<String, String>>| {
            for (k, v) in m.into_iter().flatten() {
                out.entry(k.clone()).or_insert_with(|| v.clone());
            }
        };
        if let Some(env) = env {
            layer(self.private.get(env));
            layer(self.public.get(env));
        }
        layer(self.private.get(SHARED));
        layer(self.public.get(SHARED));
        out
    }
}

/// Words of a dynamic variable: `$datetime "dd MMM yyyy" 1 d` → `$datetime`, `dd MMM yyyy`, `1`, `d`.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' || c == '\'' {
            chars.next();
            out.push(chars.by_ref().take_while(|&d| d != c).collect());
        } else {
            let mut w = String::new();
            while let Some(&d) = chars.peek().filter(|d| !d.is_whitespace()) {
                w.push(d);
                chars.next();
            }
            out.push(w);
        }
    }
    out
}

/// `1 d`, `-2 h` (Visual Studio's units: y, M, w, d, h, m, s, ms) as a duration.
fn offset(args: &[String]) -> Option<chrono::TimeDelta> {
    let [n, unit, ..] = args else { return args.is_empty().then(chrono::TimeDelta::zero) };
    let n: i64 = n.parse().ok()?;
    Some(match unit.as_str() {
        "y" => chrono::TimeDelta::days(365 * n),
        "M" => chrono::TimeDelta::days(30 * n),
        "w" => chrono::TimeDelta::weeks(n),
        "d" => chrono::TimeDelta::days(n),
        "h" => chrono::TimeDelta::hours(n),
        "m" => chrono::TimeDelta::minutes(n),
        "s" => chrono::TimeDelta::seconds(n),
        "ms" => chrono::TimeDelta::milliseconds(n),
        _ => return None,
    })
}

/// A .NET custom date format (`yyyy-MM-dd HH:mm:ss.fff`) as a chrono format.
pub fn dotnet_date_format(format: &str) -> String {
    const TOKENS: &[(&str, &str)] = &[
        ("yyyy", "%Y"),
        ("yy", "%y"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("M", "%-m"),
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("dd", "%d"),
        ("d", "%-d"),
        ("HH", "%H"),
        ("H", "%-H"),
        ("hh", "%I"),
        ("h", "%-I"),
        ("mm", "%M"),
        ("m", "%-M"),
        ("ss", "%S"),
        ("s", "%-S"),
        ("fff", "%3f"),
        ("ff", "%3f"),
        ("tt", "%p"),
        ("zzz", "%:z"),
        ("K", "%:z"),
    ];
    let mut out = String::new();
    let mut rest = format;
    'outer: while let Some(c) = rest.chars().next() {
        if c == '\'' || c == '"' {
            let body = &rest[1..];
            let end = body.find(c).unwrap_or(body.len());
            out.push_str(&body[..end].replace('%', "%%"));
            rest = body.get(end + 1..).unwrap_or("");
            continue;
        }
        for (token, chrono) in TOKENS {
            if let Some(r) = rest.strip_prefix(token) {
                out.push_str(chrono);
                rest = r;
                continue 'outer;
            }
        }
        if c == '%' {
            out.push_str("%%");
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn format_date<Tz: chrono::TimeZone>(at: chrono::DateTime<Tz>, format: &str) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match format {
        "rfc1123" => at.with_timezone(&chrono::Utc).format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
        "iso8601" => at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        custom => at.format(&dotnet_date_format(custom)).to_string(),
    }
}

/// The value of a `KEY=value` line of a `.env` file in `dirs` (nearest first).
fn dotenv(dirs: &[PathBuf], key: &str) -> Option<String> {
    dirs.iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join(".env")).ok()?;
        text.lines().find_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_owned())
        })
    })
}

fn random_below(n: u128) -> u128 {
    uuid::Uuid::new_v4().as_u128() % n.max(1)
}

fn dynamic(name: &str, dirs: &[PathBuf]) -> Option<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    match name {
        "$uuid" | "$random.uuid" | "$guid" => return Some(uuid::Uuid::new_v4().to_string()),
        "$timestamp" => return Some(now.as_secs().to_string()),
        "$isoTimestamp" => return Some(iso8601(now.as_secs())),
        "$randomInt" | "$random.integer" | "$random.integer()" => return Some(random_below(1000).to_string()),
        _ => {}
    }
    let words = words(name);
    let (head, args) = words.split_first()?;
    match head.as_str() {
        // Visual Studio: min inclusive, max exclusive.
        "$randomInt" => {
            let [min, max] = args else { return None };
            let (min, max): (i64, i64) = (min.parse().ok()?, max.parse().ok()?);
            if max <= min {
                return Some(min.to_string());
            }
            Some((min + random_below((max - min) as u128) as i64).to_string())
        }
        "$timestamp" => Some((chrono::Utc::now() + offset(args)?).timestamp().to_string()),
        "$datetime" => {
            let (format, rest) = args.split_first()?;
            Some(format_date(chrono::Utc::now() + offset(rest)?, format))
        }
        "$localDatetime" => {
            let (format, rest) = args.split_first()?;
            Some(format_date(chrono::Local::now() + offset(rest)?, format))
        }
        "$processEnv" => std::env::var(args.first()?).ok(),
        "$dotenv" => dotenv(dirs, args.first()?),
        _ => None,
    }
}

/// `secs` since the epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC).
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Resolves `{{name}}` references. File variables may reference other
/// variables; unresolved names are collected in `missing` and left as written.
pub struct Resolver<'a> {
    pub file_vars: &'a [(String, String)],
    pub env: HashMap<String, String>,
    /// Folders searched for `.env` (`$dotenv`), nearest first.
    pub dotenv_dirs: Vec<PathBuf>,
    /// The last exchange of each named request (Visual Studio's request variables).
    pub exchanges: &'a HashMap<String, crate::HttpExchange>,
}

/// Names of the requests whose variables `text` uses: `{{login.response.body.$.token}}` → `login`.
pub fn request_variable_names(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        if let Some((name, path)) = after[..end].trim().split_once('.')
            && (path.starts_with("response.") || path.starts_with("request."))
        {
            out.insert(name.to_owned());
        }
        rest = &after[end + 2..];
    }
    out
}

/// A JSONPath subset (`$`, `.key`, `['key']`, `[0]`) applied to `value`.
fn json_path<'v>(value: &'v serde_json::Value, path: &str) -> Option<&'v serde_json::Value> {
    let mut rest = path.strip_prefix('$')?;
    let mut at = value;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '[']).unwrap_or(r.len());
            at = at.get(&r[..end])?;
            rest = &r[end..];
        } else {
            let r = rest.strip_prefix('[')?;
            let end = r.find(']')?;
            let key = &r[..end];
            let quoted = key
                .strip_prefix('\'')
                .and_then(|k| k.strip_suffix('\''))
                .or_else(|| key.strip_prefix('"').and_then(|k| k.strip_suffix('"')));
            at = match quoted {
                Some(k) => at.get(k)?,
                None => at.get(key.parse::<usize>().ok()?)?,
            };
            rest = &r[end + 1..];
        }
    }
    Some(at)
}

/// `response.body.$.token`, `response.body.*`, `response.headers.Location`, `request.body.*`, …
fn request_variable(exchange: &crate::HttpExchange, path: &str) -> Option<String> {
    let (part, rest) = path.split_once('.')?;
    let (headers, body) = match part {
        "response" => {
            let r = exchange.response.as_ref()?;
            (&r.headers, r.body.as_deref())
        }
        "request" => (&exchange.request.headers, exchange.request.body.as_deref()),
        _ => return None,
    };
    if let Some(name) = rest.strip_prefix("headers.") {
        let values: Vec<&str> =
            headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect();
        return (!values.is_empty()).then(|| values.join(", "));
    }
    let selector = rest.strip_prefix("body.")?;
    let body = body?;
    if selector == "*" {
        return Some(body.to_owned());
    }
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(match json_path(&json, selector)? {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

impl Resolver<'_> {
    fn lookup(&self, name: &str, depth: u32, missing: &mut BTreeSet<String>) -> Option<String> {
        if let Some((_, v)) = self.file_vars.iter().rev().find(|(k, _)| k == name) {
            return Some(if depth < 8 { self.substitute_inner(v, depth + 1, missing) } else { v.clone() });
        }
        if let Some(v) = self.env.get(name) {
            return Some(if depth < 8 { self.substitute_inner(v, depth + 1, missing) } else { v.clone() });
        }
        if let Some((request, path)) = name.split_once('.')
            && let Some(exchange) = self.exchanges.get(request)
        {
            return request_variable(exchange, path);
        }
        dynamic(name, &self.dotenv_dirs)
    }

    fn substitute_inner(&self, text: &str, depth: u32, missing: &mut BTreeSet<String>) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = after[..end].trim();
            match self.lookup(name, depth, missing) {
                Some(v) => out.push_str(&v),
                None => {
                    missing.insert(name.to_owned());
                    out.push_str(&rest[start..start + 2 + end + 2]);
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    pub fn substitute(&self, text: &str, missing: &mut BTreeSet<String>) -> String {
        self.substitute_inner(text, 0, missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_private_public_and_shared() {
        let envs = Environments::from_json(
            r#"{"$shared":{"v":"1","host":"shared"},"dev":{"host":"localhost","port":8080,"ssl":{"x":1}},"prod":{}}"#,
            r#"{"dev":{"token":"secret"},"$shared":{"v":"2"}}"#,
        )
        .unwrap();
        assert_eq!(envs.names(), vec!["dev", "prod"]);
        let vars = envs.variables(Some("dev"));
        assert_eq!(vars["host"], "localhost");
        assert_eq!(vars["port"], "8080");
        assert_eq!(vars["token"], "secret");
        assert_eq!(vars["v"], "2");
        assert!(!vars.contains_key("ssl"));
        assert_eq!(envs.variables(None)["host"], "shared");
    }

    #[test]
    fn substitutes_file_env_and_dynamic_variables() {
        let file_vars = vec![("base".to_owned(), "https://{{host}}".to_owned())];
        let env = HashMap::from([("host".to_owned(), "example.com".to_owned())]);
        let r = Resolver { file_vars: &file_vars, env, dotenv_dirs: Vec::new(), exchanges: &HashMap::new() };
        let mut missing = BTreeSet::new();
        assert_eq!(r.substitute("{{ base }}/a?x={{nope}}", &mut missing), "https://example.com/a?x={{nope}}");
        assert_eq!(missing.into_iter().collect::<Vec<_>>(), vec!["nope"]);
        let mut missing = BTreeSet::new();
        let id = r.substitute("{{$uuid}}", &mut missing);
        assert_eq!(id.len(), 36);
        assert!(missing.is_empty());
        assert_eq!(r.substitute("{{unterminated", &mut missing), "{{unterminated");
    }

    #[test]
    fn self_reference_terminates() {
        let file_vars = vec![("a".to_owned(), "{{a}}".to_owned())];
        let r = Resolver {
            file_vars: &file_vars,
            env: HashMap::new(),
            dotenv_dirs: Vec::new(),
            exchanges: &HashMap::new(),
        };
        let mut missing = BTreeSet::new();
        assert_eq!(r.substitute("{{a}}", &mut missing), "{{a}}");
    }

    #[test]
    fn iso_timestamps() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn finds_env_files_in_ancestors_up_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("api/v1")).unwrap();
        std::fs::write(root.join(PUBLIC_FILE), r#"{"dev":{"host":"root"}}"#).unwrap();
        std::fs::write(root.join("api").join(PRIVATE_FILE), r#"{"dev":{"token":"t"}}"#).unwrap();
        let envs = Environments::load(Some(&root.join("api/v1/a.http")), Some(root)).unwrap();
        let vars = envs.variables(Some("dev"));
        assert_eq!(vars["host"], "root");
        assert_eq!(vars["token"], "t");
        // A scratch file outside the project still sees the project's environments.
        let other = tempfile::tempdir().unwrap();
        let envs = Environments::load(Some(&other.path().join("s.http")), Some(root)).unwrap();
        assert_eq!(envs.names(), vec!["dev"]);
    }

    #[test]
    fn visual_studio_dynamic_variables() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "export API_KEY=\"abc\"\nOTHER=1\n").unwrap();
        let file_vars = Vec::new();
        let r = Resolver {
            file_vars: &file_vars,
            env: HashMap::new(),
            dotenv_dirs: vec![dir.path().to_path_buf()],
            exchanges: &HashMap::new(),
        };
        let mut missing = BTreeSet::new();
        let mut sub = |t: &str| r.substitute(t, &mut missing);
        assert_eq!(sub("{{$guid}}").len(), 36);
        for _ in 0..20 {
            let n: i64 = sub("{{$randomInt 5 8}}").parse().unwrap();
            assert!((5..8).contains(&n));
        }
        assert_eq!(sub("{{$dotenv API_KEY}}"), "abc");
        let now = chrono::Utc::now().timestamp();
        let tomorrow: i64 = sub("{{$timestamp 1 d}}").parse().unwrap();
        assert!((tomorrow - now - 86_400).abs() < 5);
        assert!(sub("{{$datetime rfc1123}}").ends_with(" GMT"));
        assert!(sub("{{$datetime iso8601}}").ends_with('Z'));
        let custom = sub("{{$datetime \"yyyy-MM-dd'T'HH\"}}");
        assert_eq!(custom.len(), 13, "{custom}");
        assert_eq!(&custom[10..11], "T");
        sub("{{$dotenv MISSING}}");
        sub("{{$randomInt 1}}");
        assert_eq!(missing.into_iter().collect::<Vec<_>>(), ["$dotenv MISSING", "$randomInt 1"]);
    }

    #[test]
    fn dotnet_date_formats() {
        assert_eq!(dotnet_date_format("yyyy-MM-dd HH:mm:ss.fff"), "%Y-%m-%d %H:%M:%S.%3f");
        assert_eq!(dotnet_date_format("dddd, d MMMM 'at' h tt"), "%A, %-d %B at %-I %p");
        assert_eq!(dotnet_date_format("100%"), "100%%");
    }

    #[test]
    fn reads_the_visual_studio_user_file_as_private() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PUBLIC_FILE), r#"{"dev":{"host":"h","token":"public"}}"#).unwrap();
        std::fs::write(dir.path().join(USER_FILE), r#"{"dev":{"token":"mine"}}"#).unwrap();
        let envs = Environments::load(Some(&dir.path().join("a.http")), Some(dir.path())).unwrap();
        let vars = envs.variables(Some("dev"));
        assert_eq!((vars["host"].as_str(), vars["token"].as_str()), ("h", "mine"));
    }

    #[test]
    fn visual_studio_request_variables() {
        use crate::{HistoryEntry, HttpExchange, HttpResponse, PreparedRequest};
        let text = "# @name login\nPOST /login\n\n###\nGET /me\nAuthorization: Bearer {{login.response.body.$.token}}\nX-Id: {{login.response.body.$.items[1]['id']}}\nX-Loc: {{login.response.headers.location}}\nX-User: {{user.response.body.*}}";
        assert_eq!(request_variable_names(text).into_iter().collect::<Vec<_>>(), ["login", "user"]);
        let login = HttpExchange {
            entry: HistoryEntry {
                id: "1".into(),
                time_ms: 0.0,
                name: Some("login".into()),
                method: "POST".into(),
                url: "/login".into(),
                status: Some(200),
                total_ms: None,
                file: None,
                error: None,
            },
            request: PreparedRequest {
                name: None,
                method: "POST".into(),
                url: "/login".into(),
                headers: vec![],
                body: None,
            },
            response: Some(HttpResponse {
                status: 200,
                status_text: "OK".into(),
                version: "HTTP/1.1".into(),
                headers: vec![("Location".into(), "/home".into())],
                content_type: None,
                body: Some(r#"{"token":"t0k","items":[{"id":1},{"id":2}]}"#.into()),
                size: 0.0,
                truncated: false,
                ttfb_ms: 0.0,
                total_ms: 0.0,
            }),
        };
        let exchanges = HashMap::from([("login".to_owned(), login)]);
        let file_vars = Vec::new();
        let r = Resolver { file_vars: &file_vars, env: HashMap::new(), dotenv_dirs: Vec::new(), exchanges: &exchanges };
        let mut missing = BTreeSet::new();
        assert_eq!(r.substitute("{{login.response.body.$.token}}", &mut missing), "t0k");
        assert_eq!(r.substitute("{{login.response.body.$.items[1]['id']}}", &mut missing), "2");
        assert_eq!(r.substitute("{{login.response.headers.location}}", &mut missing), "/home");
        assert!(r.substitute("{{login.response.body.*}}", &mut missing).starts_with("{\"token\""));
        assert!(missing.is_empty(), "{missing:?}");
        // A request that has not run yet is reported as missing.
        r.substitute("{{user.response.body.*}}", &mut missing);
        assert!(missing.contains("user.response.body.*"));
    }
}
