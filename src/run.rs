//! Running the picked legs' steps on this host, in the workspace itself.

use crate::expr::{self, Ctx, Status};
use crate::plan::Selection;
use crate::process::{arch, execute, minutes, print_tail, StepFiles};
use crate::workflow::{yaml_text, Leg, Os, Workflow};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub struct Options {
    pub workspace: PathBuf,
    pub run_dir: PathBuf,
    pub keep_going: bool,
    pub quiet: bool,
    pub host: Os,
    /// `--affected`: leave out tool steps whose inputs did not change.
    pub affected: Option<crate::affected::Affected>,
    /// `--cache`: reuse steps that passed on the same inputs.
    pub cache: Option<crate::cache::Cache>,
    /// `--parallel`: the number of legs that run at the same time.
    pub parallel: usize,
}

#[derive(Serialize)]
pub struct StepReport {
    pub index: usize,
    pub name: String,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct LegReport {
    pub key: String,
    pub name: String,
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<usize>,
    pub steps: Vec<StepReport>,
}

pub(crate) fn skipped(leg: &Leg, reason: String) -> LegReport {
    LegReport {
        key: leg.key.clone(),
        name: leg.name.clone(),
        result: "skipped",
        reason: Some(reason),
        seconds: 0.0,
        failed_step: None,
        steps: Vec::new(),
    }
}

fn eval_env(
    source: &BTreeMap<String, serde_yaml::Value>,
    env: &mut Map<String, Value>,
    contexts: &mut Map<String, Value>,
) {
    for (k, v) in source {
        contexts.insert("env".into(), Value::Object(env.clone()));
        let ctx = Ctx {
            contexts,
            status: Status::Success,
        };
        let text = yaml_text(v);
        let value = expr::interpolate(&text, &ctx).unwrap_or(text);
        env.insert(k.clone(), Value::String(value));
    }
    contexts.insert("env".into(), Value::Object(env.clone()));
}

pub(crate) fn run_leg(
    leg: &Leg,
    workflow: &Workflow,
    base: &Map<String, Value>,
    needs: &Map<String, Value>,
    sel: &Selection,
    opts: &Options,
) -> (LegReport, Value) {
    let started = Instant::now();
    let dir = opts
        .run_dir
        .join(leg.key.replace(['#', '/', '\\', ' '], "-"));
    let temp = dir.join("tmp");
    if let Err(e) = private_dir(&temp) {
        return (
            skipped(leg, format!("cannot create {}: {e}", temp.display())),
            Value::Null,
        );
    }
    let mut contexts = base.clone();
    contexts.insert("matrix".into(), Value::Object(leg.matrix.clone()));
    let job_needs: Map<String, Value> = leg
        .job
        .needs
        .list()
        .iter()
        .filter_map(|n| needs.get(n).map(|v| (n.clone(), v.clone())))
        .collect();
    let needs_ok = job_needs.values().all(|v| v["result"] == "success");
    contexts.insert("needs".into(), Value::Object(job_needs));
    contexts.insert("runner".into(), json!({
        "os": opts.host.runner_name(), "arch": arch(), "temp": temp, "name": "localci", "tool_cache": temp,
    }));
    contexts.insert("job".into(), json!({ "status": "success" }));
    if let Some(Value::Object(github)) = contexts.get_mut("github") {
        github.insert("job".into(), Value::String(leg.job.id.clone()));
    }
    let mut env = Map::new();
    env.insert("LOCALCI".into(), Value::String("true".into()));
    eval_env(&workflow.env, &mut env, &mut contexts);
    eval_env(&leg.job.env, &mut env, &mut contexts);

    if let (false, Some(cond)) = (sel.ignore_if, &leg.job.condition) {
        let status = if needs_ok {
            Status::Success
        } else {
            Status::Failure
        };
        let ctx = Ctx {
            contexts: &contexts,
            status,
        };
        let text = yaml_text(cond);
        match expr::condition(&text, &ctx) {
            Ok(true) => {}
            Ok(false) => {
                return (
                    skipped(
                        leg,
                        format!("if: {text} is false here (--ignore-if runs it)"),
                    ),
                    Value::Null,
                )
            }
            Err(e) => {
                return (
                    skipped(leg, format!("cannot evaluate if: {e}")),
                    Value::Null,
                )
            }
        }
    } else if !needs_ok && !sel.ignore_if {
        return (
            skipped(leg, "a needed job did not succeed".into()),
            Value::Null,
        );
    }
    for unsupported in [
        ("container", leg.job.container.is_some()),
        ("uses", leg.job.uses.is_some()),
    ] {
        if unsupported.1 {
            eprintln!(
                "localci: {} uses `{}`, which is ignored; steps run on this host",
                leg.key, unsupported.0
            );
        }
    }
    let mut _services = None;
    if let (Some(defs), false) = (&leg.job.services, sel.no_services) {
        match crate::services::start(
            defs,
            &Ctx {
                contexts: &contexts,
                status: Status::Success,
            },
            opts.quiet,
        ) {
            Ok((running, context)) => {
                _services = Some(running);
                contexts.insert(
                    "job".into(),
                    json!({ "status": "success", "services": context }),
                );
            }
            Err(e) => {
                let mut report = skipped(leg, e);
                report.result = "failure";
                return (report, Value::Null);
            }
        }
    }

    let deadline = minutes(leg.job.timeout_minutes.as_ref()).map(|d| started + d);
    let mut status = Status::Success;
    let mut steps_ctx = Map::new();
    let mut extra_path: Vec<String> = Vec::new();
    let mut reports = Vec::new();
    let mut failed_step = None;
    let total = leg.job.steps.len();
    for (i, step) in leg.job.steps.iter().enumerate() {
        let index = i + 1;
        let label = step.label();
        let mut report = StepReport {
            index,
            name: label.clone(),
            outcome: "skipped",
            reason: None,
            seconds: 0.0,
            log: None,
        };
        contexts.insert("steps".into(), Value::Object(steps_ctx.clone()));
        contexts.insert("env".into(), Value::Object(env.clone()));
        if let Some(reason) = sel.leaves_out(leg, index, step) {
            report.reason = Some(reason.into());
            reports.push(report);
            continue;
        }
        let cond = step.condition.as_ref().map(yaml_text).unwrap_or_default();
        match expr::condition(
            &cond,
            &Ctx {
                contexts: &contexts,
                status,
            },
        ) {
            Ok(true) => {}
            Ok(false) => {
                report.reason = Some(if cond.is_empty() {
                    "an earlier step failed".into()
                } else {
                    format!("if: {cond}")
                });
                reports.push(report);
                continue;
            }
            Err(e) => {
                report.reason = Some(format!("cannot evaluate if: {e}"));
                reports.push(report);
                continue;
            }
        }
        if let Some(reason) = opts
            .affected
            .as_ref()
            .and_then(|a| a.leaves_out(leg, index, step))
        {
            eprintln!(
                "── {} · {index}/{total} {label}: skipped, {reason}",
                leg.name
            );
            report.reason = Some(reason);
            reports.push(report);
            continue;
        }
        let mut step_env = env.clone();
        let mut step_contexts = contexts.clone();
        eval_env(&step.env, &mut step_env, &mut step_contexts);
        let cache_key = opts.cache.as_ref().map(|_| {
            let script = expr::interpolate(
                step.run.as_deref().unwrap_or_default(),
                &Ctx {
                    contexts: &step_contexts,
                    status,
                },
            )
            .unwrap_or_default();
            let shell = step
                .shell
                .as_deref()
                .or(leg.job.defaults.run.shell.as_deref())
                .or(workflow.defaults.run.shell.as_deref());
            let workdir = step
                .working_directory
                .as_deref()
                .or(leg.job.defaults.run.working_directory.as_deref())
                .or(workflow.defaults.run.working_directory.as_deref());
            let path = format!(
                "{}:{}",
                extra_path.join(":"),
                step_env
                    .get("PATH")
                    .map(expr::to_text)
                    .or_else(|| std::env::var("PATH").ok())
                    .unwrap_or_default()
            );
            let versions = opts
                .cache
                .as_ref()
                .map(|c| c.tool_versions(&script, workdir.unwrap_or("."), &path))
                .unwrap_or_default();
            (
                crate::cache::Cache::key(
                    &script,
                    shell,
                    workdir,
                    &step_env,
                    opts.host.runner_name(),
                    &path,
                    &versions,
                ),
                script,
                workdir.unwrap_or(".").to_string(),
            )
        });
        let hit = match (&opts.cache, &cache_key) {
            (Some(cache), Some((key, script, workdir))) if step.run.is_some() => {
                cache.lookup(key, script, workdir)
            }
            _ => None,
        };
        if let Some(entry) = hit {
            for (k, v) in &entry.env {
                env.insert(k.clone(), Value::String(v.clone()));
            }
            extra_path.splice(0..0, entry.path.iter().cloned());
            if let Some(id) = &step.id {
                steps_ctx.insert(id.clone(), json!({ "outputs": entry.outputs, "outcome": "success", "conclusion": "success" }));
            }
            eprintln!(
                "── {} · {index}/{total} {label}: passed before on the same inputs (cache)",
                leg.name
            );
            report.outcome = "success";
            report.reason = Some("cached: passed before on the same inputs".into());
            reports.push(report);
            continue;
        }
        eprintln!("\n── {} · {index}/{total} {label}", leg.name);
        let step_start = Instant::now();
        let files = StepFiles::new(&dir, index);
        let log = dir.join(format!("step-{index}.log"));
        let outcome = execute(
            leg,
            workflow,
            step,
            &step_env,
            &step_contexts,
            &extra_path,
            &files,
            &log,
            &temp,
            deadline,
            opts,
        );
        report.seconds = step_start.elapsed().as_secs_f64();
        report.log = Some(log.clone());
        let (outputs, new_env, new_path) = files.read();
        if let (Ok(()), Some(cache), Some((key, _, _))) = (&outcome, &opts.cache, &cache_key) {
            let entry = crate::cache::Entry {
                outputs: outputs.clone(),
                env: new_env.clone(),
                path: new_path.clone(),
                ..Default::default()
            };
            cache.record(key, entry);
        }
        for (k, v) in new_env {
            env.insert(k, Value::String(v));
        }
        extra_path.splice(0..0, new_path);
        let success = outcome.is_ok();
        let continue_on_error = step.continue_on_error.as_ref().is_some_and(|v| {
            expr::truthy(
                &expr::eval(
                    &yaml_text(v).replace("${{", "(").replace("}}", ")"),
                    &Ctx {
                        contexts: &contexts,
                        status,
                    },
                )
                .unwrap_or(Value::Bool(false)),
            )
        });
        let conclusion = if success || continue_on_error {
            "success"
        } else {
            "failure"
        };
        if let Some(id) = &step.id {
            steps_ctx.insert(id.clone(), json!({ "outputs": outputs, "outcome": if success { "success" } else { "failure" }, "conclusion": conclusion }));
        }
        match outcome {
            Ok(()) => {
                report.outcome = "success";
                eprintln!("   ✓ {:.1}s", report.seconds);
            }
            Err(why) => {
                report.outcome = "failure";
                eprintln!(
                    "   ✗ {why} after {:.1}s  log: {}",
                    report.seconds,
                    log.display()
                );
                if opts.quiet {
                    print_tail(&log, 40);
                }
                report.reason = Some(why);
                if conclusion == "failure" {
                    status = Status::Failure;
                    failed_step.get_or_insert(index);
                    if let Some(Value::Object(job)) = contexts.get_mut("job") {
                        job.insert("status".into(), json!("failure"));
                    }
                }
            }
        }
        reports.push(report);
    }
    contexts.insert("steps".into(), Value::Object(steps_ctx));
    let ctx = Ctx {
        contexts: &contexts,
        status,
    };
    let outputs: Map<String, Value> = leg
        .job
        .outputs
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                Value::String(expr::interpolate(v, &ctx).unwrap_or_default()),
            )
        })
        .collect();
    let result = if status == Status::Success {
        "success"
    } else {
        "failure"
    };
    let report = LegReport {
        key: leg.key.clone(),
        name: leg.name.clone(),
        result,
        reason: None,
        seconds: started.elapsed().as_secs_f64(),
        failed_step,
        steps: reports,
    };
    (report, Value::Object(outputs))
}

/// Creates `path` and its missing parents for this user only, as a runner's
/// `RUNNER_TEMP` is. Tests that keep private files refuse a group-writable parent.
pub fn private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// A warning when other users can write to the temp folder and it is not sticky,
/// which makes tests that create private files fail.
pub fn temp_warning() -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let temp = std::env::temp_dir();
        let mode = fs::metadata(&temp).ok()?.permissions().mode();
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            return Some(format!(
                "{} (mode {:o}) is writable by other users and not sticky; tests that create private files may fail. Use a folder with mode 700 as TMPDIR.",
                temp.display(),
                mode & 0o7777
            ));
        }
    }
    None
}
