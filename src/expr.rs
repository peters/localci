//! The subset of the GitHub Actions expression language that workflows use in practice:
//! contexts, literals, `! == != < <= > >= && ||`, indexing, and the common functions.

use serde_json::{Map, Value};

/// The job's status so far, which `success()` and `failure()` report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Success,
    Failure,
}

pub struct Ctx<'a> {
    pub contexts: &'a Map<String, Value>,
    pub status: Status,
}

/// Evaluates one expression, the text between `${{` and `}}`.
pub fn eval(src: &str, ctx: &Ctx) -> Result<Value, String> {
    let tokens = lex(src)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        ctx,
    };
    let value = parser.or()?;
    match parser.tokens.get(parser.pos) {
        None => Ok(value),
        Some(tok) => Err(format!("unexpected {tok:?} in `{src}`")),
    }
}

/// Replaces every `${{ ... }}` in `text` with its value as text.
pub fn interpolate(text: &str, ctx: &Ctx) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let inner = &rest[start + 3..];
        let end = closing(inner).ok_or_else(|| format!("unterminated `${{{{` in `{text}`"))?;
        out.push_str(&to_text(&eval(&inner[..end], ctx)?));
        rest = &inner[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Evaluates an `if:` value. Without a status function it implies `success() && (...)`.
pub fn condition(src: &str, ctx: &Ctx) -> Result<bool, String> {
    let expr = src.replace("${{", "(").replace("}}", ")");
    if expr.trim().is_empty() {
        return Ok(ctx.status == Status::Success);
    }
    let lower = expr.to_ascii_lowercase();
    let explicit = ["success(", "failure(", "always(", "cancelled("]
        .iter()
        .any(|f| lower.contains(f));
    if !explicit && ctx.status != Status::Success {
        return Ok(false);
    }
    Ok(truthy(&eval(&expr, ctx)?))
}

/// The offset of the `}}` that closes an expression, skipping string literals.
fn closing(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => in_str = !in_str,
            b'}' if !in_str && bytes.get(i + 1) == Some(&b'}') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn to_num(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else if let Some(hex) = t.strip_prefix("0x") {
                i64::from_str_radix(hex, 16).map_or(f64::NAN, |n| n as f64)
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.eq_ignore_ascii_case(y),
        (Value::Array(_) | Value::Object(_), _) | (_, Value::Array(_) | Value::Object(_)) => a == b,
        (Value::Null, Value::Null) => true,
        _ => to_num(a) == to_num(b),
    }
}

fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Value::String(x), Value::String(y)) = (a, b) {
        return Some(x.to_lowercase().cmp(&y.to_lowercase()));
    }
    to_num(a).partial_cmp(&to_num(b))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    Op(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err(format!("unterminated string in `{src}`")),
                    Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                        s.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c.is_ascii_digit() || (c == '-' && next.is_some_and(|n| n.is_ascii_digit())) {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let n = to_num(&Value::String(text.clone()));
            if n.is_nan() {
                return Err(format!("bad number `{text}`"));
            }
            out.push(Tok::Num(n));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-')
            {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            let two: String = [c, next.unwrap_or(' ')].iter().collect();
            let op = match two.as_str() {
                "==" | "!=" | "<=" | ">=" | "&&" | "||" => {
                    i += 2;
                    match two.as_str() {
                        "==" => "==",
                        "!=" => "!=",
                        "<=" => "<=",
                        ">=" => ">=",
                        "&&" => "&&",
                        _ => "||",
                    }
                }
                _ => {
                    i += 1;
                    match c {
                        '!' => "!",
                        '<' => "<",
                        '>' => ">",
                        '(' => "(",
                        ')' => ")",
                        '[' => "[",
                        ']' => "]",
                        '.' => ".",
                        ',' => ",",
                        '*' => "*",
                        _ => return Err(format!("unexpected `{c}` in `{src}`")),
                    }
                }
            };
            out.push(Tok::Op(op));
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    ctx: &'a Ctx<'a>,
}

impl Parser<'_> {
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.tokens.get(self.pos), Some(Tok::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(format!(
                "expected `{op}`, found {:?}",
                self.tokens.get(self.pos)
            ))
        }
    }

    fn or(&mut self) -> Result<Value, String> {
        let mut left = self.and()?;
        while self.eat("||") {
            let right = self.and()?;
            if !truthy(&left) {
                left = right;
            }
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Value, String> {
        let mut left = self.equality()?;
        while self.eat("&&") {
            let right = self.equality()?;
            if truthy(&left) {
                left = right;
            }
        }
        Ok(left)
    }

    fn equality(&mut self) -> Result<Value, String> {
        let mut left = self.comparison()?;
        loop {
            let negate = if self.eat("==") {
                false
            } else if self.eat("!=") {
                true
            } else {
                return Ok(left);
            };
            let right = self.comparison()?;
            left = Value::Bool(loose_eq(&left, &right) != negate);
        }
    }

    fn comparison(&mut self) -> Result<Value, String> {
        use std::cmp::Ordering::{Greater, Less};
        let mut left = self.unary()?;
        loop {
            let op = ["<=", ">=", "<", ">"].into_iter().find(|op| self.eat(op));
            let Some(op) = op else { return Ok(left) };
            let right = self.unary()?;
            let ord = compare(&left, &right);
            left = Value::Bool(match op {
                "<" => ord == Some(Less),
                ">" => ord == Some(Greater),
                "<=" => matches!(ord, Some(o) if o != Greater),
                _ => matches!(ord, Some(o) if o != Less),
            });
        }
    }

    fn unary(&mut self) -> Result<Value, String> {
        if self.eat("!") {
            return Ok(Value::Bool(!truthy(&self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Value, String> {
        let mut value = self.primary()?;
        loop {
            if self.eat(".") {
                if self.eat("*") {
                    value = star(&value);
                    continue;
                }
                let Some(Tok::Ident(name)) = self.tokens.get(self.pos).cloned() else {
                    return Err("expected a property name after `.`".into());
                };
                self.pos += 1;
                value = property(&value, &name);
            } else if self.eat("[") {
                if self.eat("*") {
                    self.expect("]")?;
                    value = star(&value);
                    continue;
                }
                let index = self.or()?;
                self.expect("]")?;
                value = match (&value, &index) {
                    (Value::Array(items), Value::Number(n)) => n
                        .as_u64()
                        .and_then(|n| items.get(n as usize))
                        .cloned()
                        .unwrap_or(Value::Null),
                    _ => property(&value, &to_text(&index)),
                };
            } else {
                return Ok(value);
            }
        }
    }

    fn primary(&mut self) -> Result<Value, String> {
        let tok = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or("unexpected end of expression")?;
        self.pos += 1;
        match tok {
            Tok::Str(s) => Ok(Value::String(s)),
            Tok::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => Ok(Value::from(n as i64)),
            Tok::Num(n) => Ok(serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)),
            Tok::Op("(") => {
                let v = self.or()?;
                self.expect(")")?;
                Ok(v)
            }
            Tok::Ident(name) => {
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.or()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    return call(&name, &args, self.ctx);
                }
                Ok(match name.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    "null" => Value::Null,
                    _ => lookup(self.ctx.contexts, &name),
                })
            }
            Tok::Op(op) => Err(format!("unexpected `{op}`")),
        }
    }
}

fn lookup(map: &Map<String, Value>, name: &str) -> Value {
    map.get(name)
        .or_else(|| {
            map.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
        .cloned()
        .unwrap_or(Value::Null)
}

fn property(value: &Value, name: &str) -> Value {
    match value {
        Value::Object(map) => lookup(map, name),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| property(item, name)).collect())
        }
        _ => Value::Null,
    }
}

fn star(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Array(map.values().cloned().collect()),
        Value::Array(_) => value.clone(),
        _ => Value::Array(Vec::new()),
    }
}

fn call(name: &str, args: &[Value], ctx: &Ctx) -> Result<Value, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let text = |i: usize| to_text(&arg(i)).to_lowercase();
    Ok(match name.to_ascii_lowercase().as_str() {
        "success" => Value::Bool(ctx.status == Status::Success),
        "failure" => Value::Bool(ctx.status == Status::Failure),
        "always" => Value::Bool(true),
        "cancelled" => Value::Bool(false),
        "contains" => Value::Bool(match arg(0) {
            Value::Array(items) => items.iter().any(|item| loose_eq(item, &arg(1))),
            _ => text(0).contains(&text(1)),
        }),
        "startswith" => Value::Bool(text(0).starts_with(&text(1))),
        "endswith" => Value::Bool(text(0).ends_with(&text(1))),
        "format" => Value::String(format_args_text(
            &to_text(&arg(0)),
            &args[1.min(args.len())..],
        )),
        "join" => {
            let sep = if args.len() > 1 {
                to_text(&arg(1))
            } else {
                ",".into()
            };
            match arg(0) {
                Value::Array(items) => {
                    Value::String(items.iter().map(to_text).collect::<Vec<_>>().join(&sep))
                }
                other => Value::String(to_text(&other)),
            }
        }
        "tojson" => Value::String(serde_json::to_string_pretty(&arg(0)).unwrap_or_default()),
        "fromjson" => {
            serde_json::from_str(&to_text(&arg(0))).map_err(|e| format!("fromJSON: {e}"))?
        }
        // Only cache keys use it, and nothing is cached locally.
        "hashfiles" => Value::String(String::new()),
        _ => return Err(format!("unsupported function `{name}`")),
    })
}

fn format_args_text(fmt: &str, args: &[Value]) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                if let Ok(i) = digits.parse::<usize>() {
                    out.push_str(&args.get(i).map(to_text).unwrap_or_default());
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_with(v: Value, status: Status) -> (Map<String, Value>, Status) {
        (v.as_object().unwrap().clone(), status)
    }

    fn check(expr: &str, contexts: Value) -> Value {
        let (map, status) = ctx_with(contexts, Status::Success);
        eval(
            expr,
            &Ctx {
                contexts: &map,
                status,
            },
        )
        .unwrap()
    }

    #[test]
    fn operators_and_contexts() {
        let c = json!({"github": {"ref": "refs/heads/main", "event_name": "push"},
                       "matrix": {"os": "macos-latest", "shard": "libs"},
                       "steps": {"rust-cache": {"outputs": {"cache-hit": "true"}}}});
        assert_eq!(
            check("github.ref == 'refs/heads/main'", c.clone()),
            json!(true)
        );
        assert_eq!(
            check(
                "startsWith(matrix.os, 'macos-') && matrix.shard == 'libs'",
                c.clone()
            ),
            json!(true)
        );
        assert_eq!(
            check("steps.rust-cache.outputs.cache-hit", c.clone()),
            json!("true")
        );
        assert_eq!(
            check(
                "github.event_name == 'pull_request' && 'a' || 'b'",
                c.clone()
            ),
            json!("b")
        );
        assert_eq!(check("!cancelled()", c.clone()), json!(true));
        assert_eq!(
            check("github.event.pull_request.number || github.ref", c.clone()),
            json!("refs/heads/main")
        );
        assert_eq!(
            check("format('{0}-{1}', matrix.os, 1)", c.clone()),
            json!("macos-latest-1")
        );
        assert_eq!(
            check("contains(fromJSON('[\"a\",\"b\"]'), 'B')", c.clone()),
            json!(true)
        );
        assert_eq!(check("1 < 2 && '10' == 10", c), json!(true));
    }

    #[test]
    fn interpolation_and_conditions() {
        let (map, _) = ctx_with(json!({"matrix": {"name": "Linux x64"}}), Status::Success);
        let ctx = Ctx {
            contexts: &map,
            status: Status::Success,
        };
        assert_eq!(
            interpolate("Tests (${{ matrix.name }}) ${{ '}}' }}", &ctx).unwrap(),
            "Tests (Linux x64) }}"
        );
        assert!(condition("${{ !cancelled() }}", &ctx).unwrap());
        let failed = Ctx {
            contexts: &map,
            status: Status::Failure,
        };
        assert!(!condition("matrix.name == 'Linux x64'", &failed).unwrap());
        assert!(condition("always() && matrix.name == 'Linux x64'", &failed).unwrap());
        assert!(condition("failure()", &failed).unwrap());
    }
}
