//! `--affected`: leave out build and test steps whose inputs did not change since the
//! base commit. Only steps made of known tool commands are ever left out (ADR-0009).

use crate::workflow::{Leg, Step};
use serde_yaml::Value as Yaml;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
};

/// A build tool, the files it reads, and the commands that start it.
pub struct Tool {
    pub name: &'static str,
    commands: &'static [&'static str],
    extensions: &'static [&'static str],
    files: &'static [&'static str],
    dirs: &'static [&'static str],
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "Rust",
        commands: &["cargo"],
        extensions: &["rs"],
        files: &[
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            "rust-toolchain",
            "rust-toolchain.toml",
            "clippy.toml",
            ".clippy.toml",
            "rustfmt.toml",
            ".rustfmt.toml",
            "deny.toml",
        ],
        dirs: &[".cargo/"],
    },
    Tool {
        name: "Node",
        commands: &["npm", "npx", "pnpm", "yarn", "bun", "node"],
        extensions: &[
            "js", "mjs", "cjs", "ts", "tsx", "jsx", "json", "css", "scss", "html", "vue", "svelte",
        ],
        files: &[
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lockb",
            ".nvmrc",
            ".npmrc",
        ],
        dirs: &[],
    },
    Tool {
        name: ".NET",
        commands: &["dotnet"],
        extensions: &[
            "cs", "fs", "vb", "csproj", "fsproj", "vbproj", "props", "targets", "sln", "slnx",
            "json", "razor", "cshtml", "xaml", "resx", "config",
        ],
        files: &["global.json", "nuget.config", "NuGet.Config"],
        dirs: &[],
    },
    Tool {
        name: "Go",
        commands: &["go", "golangci-lint", "gofmt"],
        extensions: &["go"],
        files: &["go.mod", "go.sum", "go.work", "go.work.sum"],
        dirs: &[],
    },
    Tool {
        name: "Gradle",
        commands: &["gradle", "./gradlew", "gradlew"],
        extensions: &["kt", "kts", "java", "gradle", "xml", "properties", "toml"],
        files: &[],
        dirs: &["gradle/"],
    },
    Tool {
        name: "Swift",
        commands: &["swift", "xcodebuild"],
        extensions: &[
            "swift",
            "m",
            "mm",
            "h",
            "c",
            "plist",
            "xcconfig",
            "pbxproj",
            "xcscheme",
            "storyboard",
            "xib",
            "entitlements",
        ],
        files: &["Package.swift", "Package.resolved"],
        dirs: &[],
    },
];

/// Commands that read no project files, so they never make a step "unknown".
const BUILTINS: &[&str] = &[
    "echo", "printf", "set", "local", "return", "exit", "shift", "export", "cd", "test", "[", "[[",
    "true", "false", "unset", ":", "case", "declare", "readonly", "trap", "wait", "sleep", "pushd",
    "popd", "mkdir", "rustup",
];

/// Shell words that start or end a block; they are not commands.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "esac", ";;", "{",
    "}", "(", ")", "!", "time",
];

pub struct Affected {
    root: PathBuf,
    base: String,
    changed: BTreeSet<String>,
    /// Steps of the base workflow as YAML text, per job id.
    old_steps: HashMap<String, Vec<String>>,
    /// Steps of the current workflow as YAML text, per job id.
    new_steps: HashMap<String, Vec<String>>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn steps_by_job(text: &str) -> HashMap<String, Vec<String>> {
    let doc: Yaml = serde_yaml::from_str(text).unwrap_or(Yaml::Null);
    let mut out = HashMap::new();
    if let Some(Yaml::Mapping(jobs)) = doc.get("jobs") {
        for (id, job) in jobs {
            let steps = match job.get("steps") {
                Some(Yaml::Sequence(steps)) => steps
                    .iter()
                    .map(|s| serde_yaml::to_string(s).unwrap_or_default())
                    .collect(),
                _ => Vec::new(),
            };
            out.insert(crate::workflow::yaml_text(id), steps);
        }
    }
    out
}

impl Affected {
    /// The files that differ from `base`: committed, staged, unstaged and untracked.
    pub fn new(root: &Path, base: &str, workflow: &Path) -> Result<Affected, String> {
        let mut changed = BTreeSet::new();
        for args in [
            vec!["diff", "--name-only", "--no-renames", base, "HEAD"],
            vec!["diff", "--name-only", "--no-renames", "HEAD"],
            vec!["ls-files", "--others", "--exclude-standard"],
        ] {
            let out = git(root, &args).ok_or_else(|| format!("git {} failed", args.join(" ")))?;
            changed.extend(out.lines().filter(|l| !l.is_empty()).map(String::from));
        }
        let rel = workflow
            .strip_prefix(root)
            .unwrap_or(workflow)
            .to_string_lossy()
            .replace('\\', "/");
        let old = git(root, &["show", &format!("{base}:{rel}")]).unwrap_or_default();
        let new = std::fs::read_to_string(workflow).unwrap_or_default();
        Ok(Affected {
            root: root.to_path_buf(),
            base: base.to_string(),
            changed,
            old_steps: steps_by_job(&old),
            new_steps: steps_by_job(&new),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn changed(&self) -> &BTreeSet<String> {
        &self.changed
    }

    /// Why this step can be left out, or `None` when it must run.
    pub fn leaves_out(&self, leg: &Leg, index: usize, step: &Step) -> Option<String> {
        self.decide(leg, index, step).ok()
    }

    /// `Ok` with the reason to leave the step out, or `Err` with the reason it runs.
    pub fn decide(&self, leg: &Leg, index: usize, step: &Step) -> Result<String, String> {
        let run = step.run.as_deref().ok_or("it is not a run step")?;
        let current = self
            .new_steps
            .get(&leg.job.id)
            .and_then(|s| s.get(index - 1))
            .ok_or("the step is not in the workflow file")?;
        if !self
            .old_steps
            .get(&leg.job.id)
            .is_some_and(|old| old.contains(current))
        {
            return Err(format!("the step changed since {}", short(&self.base)));
        }
        let usage = analyze(run, &self.root, 0)?;
        if usage.tools.is_empty() {
            return Err("it runs no known build tool".into());
        }
        if let Some(script) = usage.scripts.iter().find(|s| self.changed.contains(*s)) {
            return Err(format!("{script} changed"));
        }
        for tool in &usage.tools {
            if let Some(file) = self.changed.iter().find(|f| is_input(&self.root, tool, f)) {
                return Err(format!("{file} is a {} input", tool.name));
            }
        }
        let names: Vec<&str> = usage.tools.iter().map(|t| t.name).collect();
        Ok(format!(
            "affected: no {} inputs changed since {}",
            names.join(" or "),
            short(&self.base)
        ))
    }
}

/// Whether `file` is an input of `tool`: a source file, a config file, or a file
/// that a source file names (an `include_str!` path, a test fixture path, or any
/// string that ends with the file's last two path parts).
pub fn is_input(root: &Path, tool: &Tool, file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
    if tool.extensions.contains(&ext)
        || tool.files.contains(&name)
        || tool
            .dirs
            .iter()
            .any(|d| file.starts_with(d) || file.contains(&format!("/{d}")))
    {
        return true;
    }
    let parts: Vec<&str> = file.split('/').collect();
    let needle = if parts.len() >= 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        format!("/{file}")
    };
    let mut args: Vec<String> = vec!["grep".into(), "-l".into(), "-F".into(), "-e".into(), needle];
    // A bare name reaches a root file only from a project at the root: Cargo runs tests
    // in each crate's folder and resolves include paths from the source file.
    if parts.len() == 1 && !(tool.name == "Rust" && !root_is_rust_package(root)) {
        args.extend(["-e".into(), format!("\"{file}\"")]);
    }
    args.push("--".into());
    args.extend(tool.extensions.iter().map(|e| format!("*.{e}")));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(root, &refs).is_some_and(|out| !out.trim().is_empty())
}

fn root_is_rust_package(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .is_ok_and(|t| t.lines().any(|l| l.trim() == "[package]"))
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

/// The tools and the repository scripts that a step's commands use.
#[derive(Default)]
pub struct Usage {
    pub tools: Vec<&'static Tool>,
    pub scripts: Vec<String>,
}

/// The tools a script uses, or why localci cannot tell: a command that is not a known
/// tool or builtin, a missing script, or a command substitution.
pub fn analyze(script: &str, root: &Path, depth: usize) -> Result<Usage, String> {
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
    let mut usage = Usage::default();
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
            let Some(first) = words.next() else { continue };
            if first.contains("()") || first == "in" {
                continue;
            }
            if BUILTINS.contains(&first)
                || KEYWORDS.contains(&first)
                || functions.iter().any(|f| f == first)
            {
                continue;
            }
            if let Some(tool) = TOOLS.iter().find(|t| t.commands.contains(&first)) {
                if !usage.tools.iter().any(|u| u.name == tool.name) {
                    usage.tools.push(tool);
                }
                continue;
            }
            let path = if matches!(first, "bash" | "sh" | "source" | ".") {
                words
                    .find(|w| !w.starts_with('-'))
                    .ok_or("it reads a script from standard input")?
            } else {
                first
            };
            let file = root.join(path.trim_start_matches("./"));
            if !file.is_file() {
                return Err(format!("`{first}` is not a known tool command"));
            }
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?;
            let inner = analyze(&text, root, depth + 1).map_err(|e| format!("{path}: {e}"))?;
            usage
                .scripts
                .push(path.trim_start_matches("./").to_string());
            usage.scripts.extend(inner.scripts);
            for tool in inner.tools {
                if !usage.tools.iter().any(|u| u.name == tool.name) {
                    usage.tools.push(tool);
                }
            }
        }
    }
    Ok(usage)
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

/// The commands of one line: split at `;`, `&&`, `||` and `|`, outside quotes.
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
                if matches!(chars.peek(), Some(';')) && c == ';' {
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

    fn tools(script: &str) -> Option<Vec<&'static str>> {
        analyze(script, Path::new("/nonexistent"), 0)
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
        assert_eq!(
            tools("case \"$x\" in\n  a) python3 y ;;\n  *) cargo test ;;\nesac"),
            None
        );
        assert_eq!(
            tools("case \"$x\" in\n  a) cargo test ;;\nesac"),
            Some(vec!["Rust"])
        );
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
    }
}
