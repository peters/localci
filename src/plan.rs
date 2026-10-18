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
