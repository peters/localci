//! Reading a step's shell script: which build tools it runs, which repository scripts it
//! calls, and whether every command is known. Used by `--affected` and `--cache`.

use crate::affected::{Tool, TOOLS};
use std::path::Path;

/// Commands that read no project files, so they never make a step "unknown".
const BUILTINS: &[&str] = &[
    "echo", "printf", "set", "local", "return", "exit", "shift", "export", "cd", "test", "[", "[[",
    "true", "false", "unset", ":", "case", "declare", "readonly", "trap", "wait", "sleep", "pushd",
    "popd", "mkdir", "rustup",
];

/// Shell words that start or end a block; they are not commands.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "until", "do", "done", "esac", ";;", "{", "}",
    "(", ")", "!", "time",
];

/// The files that a step can write to change later steps.
const STEP_FILES: &[&str] = &["GITHUB_ENV", "GITHUB_PATH", "GITHUB_OUTPUT", "GITHUB_STATE"];

/// The tools and the repository scripts that a step's commands use.
#[derive(Default)]
pub struct Usage {
    pub tools: Vec<&'static Tool>,
    pub scripts: Vec<String>,
    pub writes_step_files: bool,
}

/// The tools a script uses, or why localci cannot tell: a command that is not a known
/// tool or builtin, a script outside the repository, or a command substitution.
/// Relative script paths resolve from `workdir`, relative to `root`.
pub fn analyze(script: &str, root: &Path, workdir: &str, depth: usize) -> Result<Usage, String> {
    if depth > 3 {
        return Err("scripts call scripts more than three levels deep".into());
    }
    if script.contains('`') || script.contains("$(") || script.contains("<(") {
        return Err("it uses a command substitution".into());
    }
    let joined = script.replace("\\\r\n", " ").replace("\\\n", " ");
    let functions: Vec<String> = joined
        .lines()
        .filter_map(|l| {
            let t = l.trim().trim_start_matches("function ").trim();
            let name = t.split_once("()").map(|(n, _)| n.trim())?;
            name.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                .then(|| name.to_string())
        })
        .collect();
    let mut usage = Usage {
        writes_step_files: STEP_FILES.iter().any(|f| joined.contains(f)),
        ..Usage::default()
    };
    for line in joined.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for segment in segments(line) {
            let words = crate::shell::split(&segment);
            // `for x in a b` and `select x in a b` name a variable and a list, not a command.
            if matches!(words.first().map(String::as_str), Some("for" | "select")) {
                continue;
            }
            let mut words = words
                .iter()
                .map(String::as_str)
                .skip_while(|w| KEYWORDS.contains(w) || is_assignment(w) || is_case_pattern(w));
            let Some(mut first) = words.next() else {
                continue;
            };
            // `name() { body` on one line: the body is a command too.
            if first.contains("()") {
                let Some(body) = words.by_ref().find(|w| *w != "{") else {
                    continue;
                };
                first = body;
            }
            if first == "in" || BUILTINS.contains(&first) || functions.iter().any(|f| f == first) {
                continue;
            }
            if let Some(tool) = TOOLS.iter().find(|t| t.commands.contains(&first)) {
                add_tool(&mut usage, tool);
                continue;
            }
            let path = if matches!(first, "bash" | "sh" | "source" | ".") {
                words
                    .find(|w| !w.starts_with('-'))
                    .ok_or("it reads a script from standard input")?
            } else {
                first
            };
            let rel = repo_path(root, workdir, path)
                .ok_or_else(|| format!("`{first}` is not a known tool command"))?;
            let text =
                std::fs::read_to_string(root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
            let inner =
                analyze(&text, root, workdir, depth + 1).map_err(|e| format!("{rel}: {e}"))?;
            usage.scripts.push(rel);
            usage.scripts.extend(inner.scripts);
            usage.writes_step_files |= inner.writes_step_files;
            for tool in inner.tools {
                add_tool(&mut usage, tool);
            }
        }
    }
    Ok(usage)
}

fn add_tool(usage: &mut Usage, tool: &'static Tool) {
    if !usage.tools.iter().any(|u| u.name == tool.name) {
        usage.tools.push(tool);
    }
}

/// A script path as a repository path, or `None` when it is not a file in the repository.
fn repo_path(root: &Path, workdir: &str, path: &str) -> Option<String> {
    if Path::new(path).is_absolute() {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let file = root.join(workdir).join(path).canonicalize().ok()?;
    if !file.is_file() {
        return None;
    }
    let rel = file.strip_prefix(&root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// A `case` branch label such as `ui)` or `*)`; the command after it still counts.
fn is_case_pattern(word: &str) -> bool {
    word.ends_with(')') && !word.contains('(')
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    })
}

/// The commands of one line: split at `;`, `&&`, `||`, `|` and `&`, outside quotes,
/// redirections and `[[ ... ]]` tests.
fn segments(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' if quote.is_none() => {
                quote = Some(c);
                current.push(c);
            }
            c if quote == Some(c) => {
                quote = None;
                current.push(c);
            }
            // `>&2`, `2>&1` and `&>file` are redirections, not command separators.
            '&' if quote.is_none()
                && (current.ends_with('>')
                    || current.ends_with('<')
                    || chars.peek() == Some(&'>')) =>
            {
                current.push(c);
            }
            // `&&` and `||` inside `[[ ... ]]` belong to the test expression.
            '|' | '&'
                if quote.is_none()
                    && current.matches("[[").count() > current.matches("]]").count() =>
            {
                current.push(c);
            }
            ';' | '|' | '&' if quote.is_none() => {
                if c == ';' && chars.peek() == Some(&';') {
                    chars.next();
                    current.push_str(" ;;");
                    continue;
                }
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tools(script: &str) -> Option<Vec<&'static str>> {
        analyze(script, Path::new("/nonexistent"), ".", 0)
            .ok()
            .map(|u| u.tools.iter().map(|t| t.name).collect())
    }

    #[test]
    fn recognizes_tool_only_scripts() {
        assert_eq!(
            tools("cargo test --locked -p a\ncargo clippy -- -D warnings"),
            Some(vec!["Rust"])
        );
        assert_eq!(
            tools("RUSTFLAGS=-Dwarnings cargo test && npm test"),
            Some(vec!["Rust", "Node"])
        );
        let shard = "set -euo pipefail\nshard=\"${SHARD:?x}\"\nrun_cargo() {\n  if [[ \"$X\" == 1 ]]; then\n    printf '%s\\n' \"cargo $*\"\n    return 0\n  fi\n  cargo \"$@\"\n}\ncase \"$shard\" in\n  ui)\n    run_cargo test -p ui\n    ;;\n  *)\n    exit 1\n    ;;\nesac\n";
        assert_eq!(tools(shard), Some(vec!["Rust"]));
    }

    #[test]
    fn unknown_commands_make_the_step_unknown() {
        assert_eq!(tools("cargo build && python3 -m unittest"), None);
        assert_eq!(tools("rev=$(git rev-parse HEAD)\ncargo test"), None);
        assert_eq!(tools("cargo test 2>&1 | tee log.txt"), None);
        assert_eq!(tools("bash scripts/missing.sh"), None);
        assert_eq!(tools("bash /etc/profile"), None);
        assert_eq!(tools("echo hi"), Some(vec![]));
        assert_eq!(
            tools("printf 'bad: %s\\n' \"$x\" >&2\ncargo test 2>&1"),
            Some(vec!["Rust"])
        );
        assert_eq!(tools("cargo test &> log.txt"), Some(vec!["Rust"]));
        assert_eq!(tools("cargo test & python3 x.py"), None);
        assert_eq!(
            tools("if [[ \"$a\" == 1 && \"$b\" != 2 || -z \"$c\" ]]; then cargo test; fi"),
            Some(vec!["Rust"])
        );
        assert_eq!(
            tools("for f in \"\" video; do\n  cargo test --features \"$f\"\ndone"),
            Some(vec!["Rust"])
        );
        assert_eq!(tools("for f in a b; do python3 x.py; done"), None);
        assert_eq!(tools("foo() { python3 gen.py; }; foo; cargo test"), None);
        assert_eq!(tools("build() { cargo build; }\nbuild"), Some(vec!["Rust"]));
        assert_eq!(
            tools("case \"$x\" in\n  a) python3 y ;;\n  *) cargo test ;;\nesac"),
            None
        );
        assert_eq!(
            tools("case \"$x\" in\n  a) cargo test ;;\nesac"),
            Some(vec!["Rust"])
        );
    }

    #[test]
    fn step_files_are_noticed() {
        let usage = analyze(
            "cargo test\necho \"x=1\" >> \"$GITHUB_ENV\"",
            Path::new("/nonexistent"),
            ".",
            0,
        )
        .ok();
        assert!(usage.is_some_and(|u| u.writes_step_files));
    }
}
