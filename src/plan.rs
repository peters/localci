//! Which legs and steps a run picks, and the plan an agent or a person reads first.

use crate::workflow::{yaml_text, Leg, Os, Step};
use serde_json::{json, Value};

#[derive(Default)]
pub struct Selection {
    /// Leg keys (`rust-test#2`), job ids, or text in a leg's name. Empty means all.
    pub targets: Vec<String>,
    /// `key=value` matrix filters.
    pub matrix: Vec<(String, String)>,
    /// Steps to leave out: text in the step's name or command, or `leg:N`.
    pub skip: Vec<String>,
    /// When set, only matching steps run.
    pub only: Vec<String>,
    /// Leave out steps that install tools or packages.
    pub skip_setup: bool,
    /// Run jobs even when their `if:` is false here.
    pub ignore_if: bool,
    /// Do not start the jobs' `services:` containers.
    pub no_services: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Run,
    Uses,
    Setup,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Run => "run",
            Kind::Uses => "uses",
            Kind::Setup => "setup",
        }
    }
}

const SETUP_COMMANDS: &[&str] = &[
    "apt-get ",
    "apt install",
    "sudo ",
    "brew install",
    "choco install",
    "winget install",
    "dnf install",
    "install-ci",
    "install-dependencies",
];

pub fn kind(step: &Step) -> Kind {
    let Some(run) = &step.run else {
        return Kind::Uses;
    };
    let name = step
        .name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let run = run.to_ascii_lowercase();
    if name.starts_with("install")
        || name.starts_with("setup")
        || SETUP_COMMANDS.iter().any(|c| run.contains(c))
    {
        Kind::Setup
    } else {
        Kind::Run
    }
}

impl Selection {
    pub fn picks(&self, leg: &Leg) -> bool {
        let name = leg.name.to_lowercase();
        let targeted = self.targets.is_empty()
            || self.targets.iter().any(|t| {
                let t = t.to_lowercase();
                t == leg.key.to_lowercase() || t == leg.job.id.to_lowercase() || name.contains(&t)
            });
        targeted
            && self.matrix.iter().all(|(k, v)| {
                leg.matrix
                    .get(k)
                    .is_some_and(|m| crate::expr::to_text(m).eq_ignore_ascii_case(v))
            })
    }

    /// `true` when `--skip` names this step.
    pub fn skips(&self, leg: &Leg, index: usize, step: &Step) -> bool {
        self.skip.iter().any(|p| step_matches(p, leg, index, step))
    }

    /// Why this step does not run, or `None` when it does.
    pub fn leaves_out(&self, leg: &Leg, index: usize, step: &Step) -> Option<&'static str> {
        let matches = |pattern: &String| step_matches(pattern, leg, index, step);
        match kind(step) {
            Kind::Uses => Some("uses: actions do not run locally"),
            _ if !self.only.is_empty() && !self.only.iter().any(matches) => Some("not in --only"),
            _ if self.skip.iter().any(matches) => Some("--skip"),
            Kind::Setup if self.skip_setup => Some("setup step (--skip-setup)"),
            _ => None,
        }
    }
}

fn step_matches(pattern: &str, leg: &Leg, index: usize, step: &Step) -> bool {
    if let Some((target, n)) = pattern.rsplit_once(':') {
        if let Ok(n) = n.trim().parse::<usize>() {
            return n == index
                && (target.eq_ignore_ascii_case(&leg.key)
                    || target.eq_ignore_ascii_case(&leg.job.id));
        }
    }
    let p = pattern.to_lowercase();
    step.label().to_lowercase().contains(&p)
        || step
            .run
            .as_deref()
            .is_some_and(|r| r.to_lowercase().contains(&p))
}

/// A step `if:` decided by the matrix and github contexts alone, before anything runs.
/// An `if:` decided before anything runs: it reads only github, vars, inputs,
/// matrix and `env.LOCALCI`. `None` when it needs the results of a run.
fn decided(cond: &str, leg: &Leg, base: &serde_json::Map<String, Value>) -> Option<bool> {
    let rest = cond.replace("env.LOCALCI", "");
    if ["steps.", "needs.", "env.", "failure(", "always(", "job."]
        .iter()
        .any(|d| rest.contains(d))
    {
        return None;
    }
    let mut contexts = base.clone();
    contexts.insert("matrix".into(), Value::Object(leg.matrix.clone()));
    contexts.insert("env".into(), json!({ "LOCALCI": "true" }));
    let ctx = crate::expr::Ctx {
        contexts: &contexts,
        status: crate::expr::Status::Success,
    };
    crate::expr::condition(cond, &ctx).ok()
}

pub fn static_if(leg: &Leg, step: &Step, base: &serde_json::Map<String, Value>) -> Option<bool> {
    decided(&yaml_text(step.condition.as_ref()?), leg, base)
}

/// The job-level `if:`, when it is decided before anything runs.
pub fn static_job_if(leg: &Leg, base: &serde_json::Map<String, Value>) -> Option<bool> {
    decided(&yaml_text(leg.job.condition.as_ref()?), leg, base)
}

/// One Debian package that a `run:` step installs with `apt` (ADR-0012).
pub struct Package {
    pub leg: String,
    pub step: usize,
    pub request: String,
    /// `run step <n>`.
    pub source: String,
}

/// A command or a word of an `apt` install that localci does not name, and the reason.
pub struct NotRead {
    pub leg: String,
    pub step: usize,
    /// The command or the word.
    pub text: String,
    pub reason: String,
}

/// The Debian packages of the picked `run:` steps, and what localci does not read.
#[derive(Default)]
pub struct Packages {
    pub found: Vec<Package>,
    pub unread: Vec<NotRead>,
}

/// The flags of an `apt` install that localci drops: they are not packages.
const APT_FLAGS: &[&str] = &["-y", "-q", "-qq", "--yes", "--no-install-recommends"];

/// The shell operators that end one command of a `run:` step.
const COMMAND_END: &[&str] = &["&&", "||", ";", "|", "\n"];

/// The packages and the text that localci does not read, of one `run:` text.
#[derive(Default)]
struct Parsed {
    packages: Vec<String>,
    /// The text and the reason.
    unread: Vec<(String, String)>,
}

/// The text after `apt-get install` or `apt install`, with an optional `sudo` first.
fn install_tail(command: &str) -> Option<&str> {
    let mut rest = command.trim();
    if let Some(after) = rest.strip_prefix("sudo ") {
        rest = after.trim_start();
    }
    ["apt-get install", "apt install"].iter().find_map(|head| {
        let after = rest.strip_prefix(head)?;
        (after.is_empty() || after.starts_with([' ', '\t'])).then(|| after.trim_start())
    })
}

/// `true` when a command runs `apt-get install` or `apt install`.
fn installs(command: &str) -> bool {
    ["apt-get install", "apt install"].iter().any(|head| {
        command.match_indices(head).any(|(at, _)| {
            let before = &command[..at];
            let after = &command[at + head.len()..];
            (before.is_empty() || before.ends_with([' ', '\t']))
                && (after.is_empty() || after.starts_with([' ', '\t']))
        })
    })
}

/// The commands of a `run:` text. A `\` at the end of a line joins the next line.
fn commands(run: &str) -> Vec<String> {
    let joined = run.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut out = Vec::new();
    let mut current = String::new();
    let mut rest = joined.as_str();
    while let Some(c) = rest.chars().next() {
        if let Some(end) = COMMAND_END.iter().find(|e| rest.starts_with(**e)) {
            out.push(std::mem::take(&mut current));
            rest = &rest[end.len()..];
        } else {
            current.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out.push(current);
    out
}

/// The packages of one `run:` text, and the text that localci does not read.
fn parse(run: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for command in commands(run) {
        let command = command.trim();
        if !installs(command) {
            continue;
        }
        // The shell completes this text during the run, so localci cannot know the packages.
        if command.contains('`') || command.contains("$(") {
            parsed.unread.push((
                command.to_string(),
                "the command has a command substitution".to_string(),
            ));
            continue;
        }
        if command.contains('$') {
            parsed.unread.push((
                command.to_string(),
                "the command has a shell variable".to_string(),
            ));
            continue;
        }
        let Some(tail) = install_tail(command) else {
            parsed.unread.push((
                command.to_string(),
                "localci reads only `apt-get install` or `apt install`, after an optional `sudo`"
                    .to_string(),
            ));
            continue;
        };
        for word in tail.split_whitespace() {
            if word.starts_with('-') {
                if !APT_FLAGS.contains(&word) {
                    parsed
                        .unread
                        .push((word.to_string(), "a flag that localci does not read".into()));
                }
            } else if word.contains('=') {
                parsed
                    .unread
                    .push((word.to_string(), "names a version".into()));
            } else if word.contains('/') || word.ends_with(".deb") {
                parsed
                    .unread
                    .push((word.to_string(), "is a path or a file".into()));
            } else {
                parsed.packages.push(word.to_string());
            }
        }
    }
    parsed
}

/// The `apt` packages of the picked `run:` steps of `legs`. A step that `--skip` names,
/// or whose `if:` is false before the run, is not in the plan, as in `setup::plan`.
/// A package is not a toolchain: `localci setup` installs no package.
pub fn apt_packages(
    legs: &[&Leg],
    base: &serde_json::Map<String, Value>,
    sel: &Selection,
) -> Packages {
    let mut out = Packages::default();
    for leg in legs {
        for (i, step) in leg.job.steps.iter().enumerate() {
            let index = i + 1;
            let Some(run) = step.run.as_deref() else {
                continue;
            };
            if sel.skips(leg, index, step) || static_if(leg, step, base) == Some(false) {
                continue;
            }
            let parsed = parse(run);
            let source = format!("run step {index}");
            for request in parsed.packages {
                out.found.push(Package {
                    leg: leg.key.clone(),
                    step: index,
                    request,
                    source: source.clone(),
                });
            }
            for (text, reason) in parsed.unread {
                out.unread.push(NotRead {
                    leg: leg.key.clone(),
                    step: index,
                    text,
                    reason,
                });
            }
        }
    }
    out
}

pub fn first_command(step: &Step) -> String {
    step.run.as_deref().map_or_else(
        || step.uses.clone().unwrap_or_default(),
        |r| {
            r.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or_default()
                .to_string()
        },
    )
}

pub fn to_json(
    legs: &[Leg],
    selection: &Selection,
    host: Os,
    base: &serde_json::Map<String, Value>,
    affected: Option<&crate::affected::Affected>,
) -> Value {
    let legs: Vec<Value> = legs
        .iter()
        .map(|leg| {
            let steps: Vec<Value> = leg
                .job
                .steps
                .iter()
                .enumerate()
                .map(|(i, step)| {
                    let decision = affected.map(|a| a.decide(leg, i + 1, step));
                    let (skip, runs) = match decision {
                        Some(Ok(skip)) => (Some(skip), None),
                        Some(Err(crate::affected::Runs::Because(why))) => (None, Some(why)),
                        _ => (None, None),
                    };
                    json!({
                        "index": i + 1,
                        "name": step.label(),
                        "id": step.id,
                        "kind": kind(step).as_str(),
                        "command": first_command(step),
                        "lines": step.run.as_deref().map_or(0, |r| r.lines().count()),
                        "if": step.condition.as_ref().map(yaml_text),
                        "leave_out": selection.leaves_out(leg, i + 1, step).map(String::from)
                            .or_else(|| (static_if(leg, step, base) == Some(false)).then(|| "if: false for this leg".to_string()))
                            .or(skip),
                        "runs_because": runs,
                    })
                })
                .collect();
            json!({
                "key": leg.key,
                "job": leg.job.id,
                "name": leg.name,
                "runs_on": leg.labels,
                "os": leg.os,
                "runnable": leg.os == Some(host),
                "selected": selection.picks(leg) && leg.os == Some(host),
                "matrix": leg.matrix,
                "needs": leg.job.needs.list(),
                "if": leg.job.condition.as_ref().map(yaml_text),
                "if_locally": static_job_if(leg, base),
                "steps": steps,
            })
        })
        .collect();
    json!({ "host": host, "legs": legs })
}

pub fn print(
    legs: &[Leg],
    selection: &Selection,
    host: Os,
    base: &serde_json::Map<String, Value>,
    affected: Option<&crate::affected::Affected>,
) {
    for leg in legs {
        let runnable = leg.os == Some(host);
        let selected = runnable && selection.picks(leg);
        let mark = if selected { "▶" } else { "·" };
        let note = if !runnable {
            format!("  (needs {})", leg.labels.join(", "))
        } else if !selected {
            "  (not selected)".into()
        } else {
            String::new()
        };
        println!("{mark} {:<22} {}{note}", leg.key, leg.name);
        if !selected {
            continue;
        }
        if let Some(cond) = &leg.job.condition {
            let verdict = match static_job_if(leg, base) {
                Some(false) => "  (false here: the job is skipped)",
                Some(true) => "  (true here)",
                None => "",
            };
            println!("      if: {}{verdict}", yaml_text(cond));
        }
        for (i, step) in leg.job.steps.iter().enumerate() {
            let k = kind(step);
            let decision = affected.map(|a| a.decide(leg, i + 1, step));
            let unaffected = decision.as_ref().and_then(|d| d.as_ref().ok().cloned());
            let status = match selection.leaves_out(leg, i + 1, step) {
                Some(reason) => format!("skip  ({reason})"),
                None if static_if(leg, step, base) == Some(false) => {
                    "skip  (if: false for this leg)".into()
                }
                None if unaffected.is_some() => {
                    format!("skip  ({})", unaffected.unwrap_or_default())
                }
                None => match decision {
                    Some(Err(crate::affected::Runs::Because(why))) if k == Kind::Run => {
                        format!("run   (runs: {why})")
                    }
                    _ => k.as_str().to_string(),
                },
            };
            let cond = step
                .condition
                .as_ref()
                .map(|c| format!("  [if: {}]", yaml_text(c)))
                .unwrap_or_default();
            println!(
                "    {:>2} {:<44} {}{cond}",
                i + 1,
                truncate(&step.label(), 44),
                status
            );
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packages(run: &str) -> Vec<String> {
        parse(run).packages
    }

    /// The text that localci does not read, in the order of the run.
    fn unread(run: &str) -> Vec<String> {
        parse(run)
            .unread
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    }

    #[test]
    fn flags_are_not_packages() {
        assert_eq!(
            packages("sudo apt-get install -y -q -qq --yes --no-install-recommends curl git"),
            ["curl", "git"]
        );
        assert!(unread("sudo apt-get install -y curl").is_empty());
        assert_eq!(packages("sudo apt-get install -s curl"), ["curl"]);
        assert_eq!(unread("sudo apt-get install -s curl"), ["-s"]);
    }

    #[test]
    fn a_continuation_joins_the_next_line() {
        let run = "sudo apt-get update\nsudo apt-get install -y \\\n  --no-install-recommends \\\n  libasound2-dev cmake\n";
        assert_eq!(packages(run), ["libasound2-dev", "cmake"]);
    }

    #[test]
    fn two_packages_in_one_line() {
        assert_eq!(
            packages("sudo apt-get update && sudo apt-get install -y libasound2-dev cmake"),
            ["libasound2-dev", "cmake"]
        );
    }

    #[test]
    fn a_variable_or_a_substitution_is_not_read() {
        let line = "sudo apt-get install -y $DEPS";
        assert_eq!(unread(line), [line]);
        assert!(packages(line).is_empty());
        assert_eq!(unread("sudo apt-get install -y $(cat deps)").len(), 1);
        assert_eq!(unread("sudo apt-get install -y `cat deps`").len(), 1);
    }

    #[test]
    fn a_pinned_version_or_a_path_is_not_a_package() {
        let run = "sudo apt-get install -y foo=1.2 ./local.deb bar";
        assert_eq!(packages(run), ["bar"]);
        assert_eq!(unread(run), ["foo=1.2", "./local.deb"]);
    }

    #[test]
    fn the_install_command_comes_first() {
        assert_eq!(packages("sudo apt-get install -y a b"), ["a", "b"]);
        assert_eq!(packages("apt install a"), ["a"]);
        assert!(packages("sudo apt-get update").is_empty());
        assert!(packages("apt-get installer foo").is_empty());
        assert!(packages("echo apt-get install foo").is_empty());
        let prefixed = "DEBIAN_FRONTEND=noninteractive apt-get install -y a";
        assert_eq!(unread(prefixed), [prefixed]);
    }
}
