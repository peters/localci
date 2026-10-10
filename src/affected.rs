//! `--affected`: leave out build and test steps whose inputs did not change since the
//! base commit. Only steps made of known tool commands are ever left out (ADR-0009).

use crate::script::analyze;
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
    pub commands: &'static [&'static str],
    extensions: &'static [&'static str],
    files: &'static [&'static str],
    dirs: &'static [&'static str],
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
        projects: &[".csproj", ".fsproj", ".vbproj"],
    },
    Tool {
        name: "Go",
        commands: &["go", "golangci-lint", "gofmt"],
        extensions: &["go"],
        files: &["go.mod", "go.sum", "go.work", "go.work.sum"],
        dirs: &[],
        projects: &["go.mod"],
    },
    Tool {
        name: "Gradle",
        commands: &["gradle", "./gradlew", "gradlew"],
        extensions: &["kt", "kts", "java", "gradle", "xml", "properties", "toml"],
        files: &[],
        dirs: &["gradle/"],
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
        projects: &["Package.swift"],
    },
];

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
        let (old_settings, old_jobs) =
            parse(&git(root, &["show", &format!("{base}:{rel}")]).unwrap_or_default());
        let (new_settings, new_jobs) =
            parse(&std::fs::read_to_string(workflow).unwrap_or_default());
        Ok(Affected {
            root: root.to_path_buf(),
            base: base.to_string(),
            changed,
            old_settings,
            new_settings,
            old_jobs,
            new_jobs,
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
        if !old.steps.contains(current) {
            return Err(format!("the step changed since {since}"));
        }
        let workdir =
            step.working_directory
                .as_deref()
                .or(leg.job.defaults.run.working_directory.as_deref());
        let usage = analyze(run, &self.root, workdir.unwrap_or("."), 0)?;
        if usage.writes_step_files {
            return Err("it writes a GITHUB_* file that later steps read".into());
        }
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
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(root, &refs).is_some_and(|out| !out.trim().is_empty())
}

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

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}
