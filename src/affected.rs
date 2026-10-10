//! `--affected`: leave out build and test steps whose inputs did not change since the
//! base commit. Only steps made of known tool commands are ever left out (ADR-0009).

use crate::script::analyze;
use crate::workflow::{Leg, Step};
use serde_yaml::Value as Yaml;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock, PoisonError},
};

/// A build tool, the files it reads, and the commands that start it.
pub struct Tool {
    pub name: &'static str,
    pub commands: &'static [&'static str],
    extensions: &'static [&'static str],
    files: &'static [&'static str],
    dirs: &'static [&'static str],
    /// Commands whose output names the toolchain version, for the step cache key.
    pub versions: &'static [&'static str],
    /// Files that mark a project folder. Every file in that folder is an input,
    /// because tests can read data from it without naming each file.
    projects: &'static [&'static str],
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
        versions: &["cargo --version", "rustc --version"],
        projects: &["Cargo.toml"],
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
        versions: &["node --version"],
        projects: &["package.json"],
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
        versions: &["dotnet --version"],
        projects: &[".csproj", ".fsproj", ".vbproj"],
    },
    Tool {
        name: "Go",
        commands: &["go", "golangci-lint", "gofmt"],
        extensions: &["go"],
        files: &["go.mod", "go.sum", "go.work", "go.work.sum"],
        dirs: &[],
        versions: &["go version"],
        projects: &["go.mod"],
    },
    Tool {
        name: "Gradle",
        commands: &["gradle", "./gradlew", "gradlew"],
        extensions: &["kt", "kts", "java", "gradle", "xml", "properties", "toml"],
        files: &[],
        dirs: &["gradle/"],
        versions: &[],
        projects: &["build.gradle", "build.gradle.kts"],
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
        versions: &["swift --version", "xcodebuild -version"],
        projects: &["Package.swift"],
    },
];

const NOT_A_TOOL: &str = "it runs no known build tool";

/// Why `--affected` runs a step.
#[derive(Debug, PartialEq, Eq)]
pub enum Runs {
    /// The step is not made of known build tool commands, so localci never leaves it out.
    NotATool,
    /// A tool step that runs for this reason.
    Because(String),
}

/// One job of a workflow: its settings without the steps, and each step, as YAML text.
#[derive(Default)]
struct JobText {
    settings: String,
    steps: Vec<String>,
}

pub struct Affected {
    root: PathBuf,
    base: String,
    changed: BTreeSet<String>,
    /// The workflow-level `env` and `defaults` in the base and now.
    old_settings: String,
    new_settings: String,
    old_jobs: HashMap<String, JobText>,
    new_jobs: HashMap<String, JobText>,
    /// The workflow's `defaults.run.working-directory`.
    default_workdir: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off"])
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn yaml(value: &Yaml) -> String {
    serde_yaml::to_string(value).unwrap_or_default()
}

fn parse(text: &str) -> (String, HashMap<String, JobText>) {
    let doc: Yaml = serde_yaml::from_str(text).unwrap_or(Yaml::Null);
    let settings = format!(
        "{}{}",
        doc.get("env").map(yaml).unwrap_or_default(),
        doc.get("defaults").map(yaml).unwrap_or_default()
    );
    let mut jobs = HashMap::new();
    if let Some(Yaml::Mapping(map)) = doc.get("jobs") {
        for (id, job) in map {
            let mut rest = job.clone();
            let steps = match rest.as_mapping_mut().and_then(|m| m.remove("steps")) {
                Some(Yaml::Sequence(steps)) => steps.iter().map(yaml).collect(),
                _ => Vec::new(),
            };
            jobs.insert(
                crate::workflow::yaml_text(id),
                JobText {
                    settings: yaml(&rest),
                    steps,
                },
            );
        }
    }
    (settings, jobs)
}

/// The commit that `--affected` compares with: `--base`, or the merge base with the
/// default branch of `origin`. `None` when neither exists.
pub fn base_commit(root: &Path, reference: Option<&str>) -> Option<String> {
    let reference = match reference {
        Some(r) => r.to_string(),
        None => {
            let head = git(
                root,
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            )
            .map(|s| s.trim().to_string());
            head.or_else(|| {
                ["origin/main", "origin/master"]
                    .iter()
                    .find(|r| git(root, &["rev-parse", "--verify", "-q", r]).is_some())
                    .map(|r| r.to_string())
            })?
        }
    };
    git(root, &["merge-base", "HEAD", &reference]).map(|s| s.trim().to_string())
}

impl Affected {
    /// The files that differ from `base`: committed, staged, unstaged and untracked.
    pub fn new(root: &Path, base: &str, workflow: &Path) -> Result<Affected, String> {
        let mut changed = BTreeSet::new();
        for args in [
            vec!["diff", "--name-only", "-z", "--no-renames", base, "HEAD"],
            vec!["diff", "--name-only", "-z", "--no-renames", "HEAD"],
            vec!["ls-files", "-z", "--others", "--exclude-standard"],
        ] {
            let out = git(root, &args).ok_or_else(|| format!("git {} failed", args.join(" ")))?;
            changed.extend(out.split('\0').filter(|l| !l.is_empty()).map(String::from));
        }
        let rel = workflow
            .strip_prefix(root)
            .unwrap_or(workflow)
            .to_string_lossy()
            .replace('\\', "/");
        let (old_settings, old_jobs) =
            parse(&git(root, &["show", &format!("{base}:{rel}")]).unwrap_or_default());
        let new_text = std::fs::read_to_string(workflow).unwrap_or_default();
        let (new_settings, new_jobs) = parse(&new_text);
        let doc: Yaml = serde_yaml::from_str(&new_text).unwrap_or(Yaml::Null);
        let default_workdir = doc
            .get("defaults")
            .and_then(|d| d.get("run"))
            .and_then(|r| r.get("working-directory"))
            .map(crate::workflow::yaml_text);
        Ok(Affected {
            root: root.to_path_buf(),
            base: base.to_string(),
            changed,
            old_settings,
            new_settings,
            old_jobs,
            new_jobs,
            default_workdir,
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
    pub fn decide(&self, leg: &Leg, index: usize, step: &Step) -> Result<String, Runs> {
        self.decide_inner(leg, index, step).map_err(|why| {
            if why == NOT_A_TOOL {
                Runs::NotATool
            } else {
                Runs::Because(why)
            }
        })
    }

    fn decide_inner(&self, leg: &Leg, index: usize, step: &Step) -> Result<String, String> {
        let run = step.run.as_deref().ok_or(NOT_A_TOOL)?;
        if step.id.is_some() {
            return Err("it has an id, so later steps can read its outputs".into());
        }
        let since = short(&self.base);
        if self.old_settings != self.new_settings {
            return Err(format!(
                "the workflow env or defaults changed since {since}"
            ));
        }
        let (Some(old), Some(new)) = (
            self.old_jobs.get(&leg.job.id),
            self.new_jobs.get(&leg.job.id),
        ) else {
            return Err(format!("the job is new since {since}"));
        };
        if old.settings != new.settings {
            return Err(format!("the job settings changed since {since}"));
        }
        let current = new
            .steps
            .get(index - 1)
            .ok_or("the step is not in the workflow file")?;
        // Steps before this one can change its environment, so they must be the same too.
        if old.steps.get(..index) != new.steps.get(..index) {
            let what = if old.steps.contains(current) {
                "a step before it"
            } else {
                "the step"
            };
            return Err(format!("{what} changed since {since}"));
        }
        let workdir = step
            .working_directory
            .as_deref()
            .or(leg.job.defaults.run.working_directory.as_deref())
            .or(self.default_workdir.as_deref());
        if workdir.is_some_and(|w| w.contains("${{")) {
            return Err("its working directory uses an expression".into());
        }
        if run.contains("needs.") {
            return Err("it reads outputs of another job".into());
        }
        // An earlier step can change this step's environment through a script it runs,
        // so a changed file that an earlier step names or calls makes this step run.
        for earlier in &leg.job.steps[..index - 1] {
            let Some(text) = earlier.run.as_deref() else {
                continue;
            };
            let mut called = Vec::new();
            crate::script::named_files(
                text,
                &self.root,
                workdir_of(leg, earlier, self).unwrap_or("."),
                0,
                &mut called,
            );
            let named = self
                .changed
                .iter()
                .find(|f| text.contains(f.as_str()) || called.contains(f));
            if let Some(file) = named {
                return Err(format!("{file}, which an earlier step runs, changed"));
            }
        }
        // A local action before this step can change its environment.
        for earlier in &leg.job.steps[..index - 1] {
            if let Some(action) = earlier.uses.as_deref().and_then(|u| u.strip_prefix("./")) {
                let action = action.trim_end_matches('/');
                if let Some(file) = self.changed.iter().find(|f| f.starts_with(action)) {
                    return Err(format!(
                        "the local action {action} before it changed ({file})"
                    ));
                }
            }
        }
        let usage = analyze(run, &self.root, workdir.unwrap_or("."), 0)?;
        if usage.writes_step_files {
            return Err("it writes a GITHUB_* file that later steps read".into());
        }
        if usage.installs {
            return Err("it installs tools or packages that later steps can need".into());
        }
        if usage.tools.is_empty() {
            return Err(NOT_A_TOOL.into());
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
            "affected: no {} inputs changed since {since}",
            names.join(" or ")
        ))
    }
}

/// Whether `file` is an input of `tool`: a source or config file, a file in one of its
/// project folders, or a file whose path a source file names (an `include_str!` path,
/// a test fixture path, or any string that ends with the file's last two path parts).
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
    if in_project(root, tool, file) {
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
    if parts.len() == 1 && !(tool.name == "Rust" && !is_project(root, tool, root)) {
        args.extend(["-e".into(), format!("\"{file}\"")]);
    }
    args.push("--".into());
    args.extend(tool.extensions.iter().map(|e| format!("*.{e}")));
    let key = format!("{}|{}|{file}", root.display(), tool.name);
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(known) = memo
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return *known;
    }
    // `git grep` exits 1 for no match. Any other failure counts as an input: a wrong
    // guess must make a step run, not hide it.
    let found = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&args)
        .output()
        .map_or(true, |out| out.status.code() != Some(1));
    memo.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, found);
    found
}

/// `is_input` answers for this process: one `git grep` for each file and tool.
static MEMO: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

/// Whether a folder that contains `file` is a project folder of `tool`.
fn in_project(root: &Path, tool: &Tool, file: &str) -> bool {
    let mut dir = root.join(file);
    while dir.pop() && dir.starts_with(root) {
        if is_project(root, tool, &dir) {
            return true;
        }
    }
    false
}

/// A Rust workspace root without `[package]` is not a project folder: its files are
/// not crate data, and Cargo runs no tests in it.
fn is_project(_root: &Path, tool: &Tool, dir: &Path) -> bool {
    tool.projects.iter().any(|marker| {
        if marker.starts_with('.') {
            std::fs::read_dir(dir).is_ok_and(|mut entries| {
                entries.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(marker)))
            })
        } else if *marker == "Cargo.toml" {
            std::fs::read_to_string(dir.join(marker))
                .is_ok_and(|t| t.lines().any(|l| l.trim() == "[package]"))
        } else {
            dir.join(marker).is_file()
        }
    })
}

/// The working directory of a step: its own, the job's default, or the workflow's.
fn workdir_of<'a>(leg: &'a Leg, step: &'a Step, affected: &'a Affected) -> Option<&'a str> {
    step.working_directory
        .as_deref()
        .or(leg.job.defaults.run.working_directory.as_deref())
        .or(affected.default_workdir.as_deref())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}
