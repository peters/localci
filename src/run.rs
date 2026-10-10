//! Running the picked legs' steps on this host, in the workspace itself.

use crate::expr::{self, Ctx, Status};
use crate::plan::Selection;
use crate::workflow::{to_json, yaml_text, Leg, Os, Workflow};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct Options {
    pub workspace: PathBuf,
    pub run_dir: PathBuf,
    pub keep_going: bool,
    pub quiet: bool,
    pub host: Os,
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

pub fn run_all(
    legs: &[&Leg],
    workflow: &Workflow,
    base: &Map<String, Value>,
    sel: &Selection,
    opts: &Options,
) -> Vec<LegReport> {
    let mut needs = Map::new();
    let mut reports = Vec::new();
    let mut failed = false;
    for leg in order(legs) {
        if failed && !opts.keep_going {
            reports.push(skipped(
                leg,
                "an earlier leg failed (use --keep-going to continue)".into(),
            ));
            continue;
        }
        let (report, outputs) = run_leg(leg, workflow, base, &needs, sel, opts);
        failed |= report.result == "failure";
        needs.insert(
            leg.job.id.clone(),
            json!({ "result": report.result, "outputs": outputs }),
        );
        reports.push(report);
    }
    reports
}

/// Picked legs with every picked dependency before its dependents.
fn order<'a, 'w>(legs: &[&'a Leg<'w>]) -> Vec<&'a Leg<'w>> {
    let mut pending: Vec<&Leg> = legs.to_vec();
    let mut out: Vec<&Leg> = Vec::new();
    while !pending.is_empty() {
        let ready = pending.iter().position(|leg| {
            leg.job
                .needs
                .list()
                .iter()
                .all(|n| !pending.iter().any(|p| &p.job.id == n))
        });
        out.push(pending.remove(ready.unwrap_or(0)));
    }
    out
}

fn skipped(leg: &Leg, reason: String) -> LegReport {
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

fn run_leg(
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
        let mut step_env = env.clone();
        let mut step_contexts = contexts.clone();
        eval_env(&step.env, &mut step_env, &mut step_contexts);
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

#[allow(clippy::too_many_arguments)]
fn execute(
    leg: &Leg,
    workflow: &Workflow,
    step: &crate::workflow::Step,
    env: &Map<String, Value>,
    contexts: &Map<String, Value>,
    extra_path: &[String],
    files: &StepFiles,
    log: &Path,
    temp: &Path,
    deadline: Option<Instant>,
    opts: &Options,
) -> Result<(), String> {
    let ctx = Ctx {
        contexts,
        status: Status::Success,
    };
    let script = expr::interpolate(step.run.as_deref().unwrap_or_default(), &ctx)?;
    let shell = step
        .shell
        .clone()
        .or_else(|| leg.job.defaults.run.shell.clone())
        .or_else(|| workflow.defaults.run.shell.clone());
    let workdir = step
        .working_directory
        .clone()
        .or_else(|| leg.job.defaults.run.working_directory.clone())
        .or_else(|| workflow.defaults.run.working_directory.clone())
        .map(|d| expr::interpolate(&d, &ctx).unwrap_or(d));
    let mut command = crate::shell::command(
        shell.as_deref(),
        &script,
        &files.dir,
        files.index,
        opts.host,
    )?;
    command.current_dir(workdir.map_or_else(|| opts.workspace.clone(), |d| opts.workspace.join(d)));
    let github = &contexts["github"];
    let text = |v: &Value| expr::to_text(v);
    command
        .env("CI", "true")
        .env("LOCALCI", "true")
        .env("GITHUB_WORKSPACE", &opts.workspace)
        .env("GITHUB_REPOSITORY", text(&github["repository"]))
        .env("GITHUB_REF", text(&github["ref"]))
        .env("GITHUB_SHA", text(&github["sha"]))
        .env("GITHUB_EVENT_NAME", text(&github["event_name"]))
        .env("GITHUB_JOB", &leg.job.id)
        .env("GITHUB_RUN_ID", "0")
        .env("RUNNER_OS", opts.host.runner_name())
        .env("RUNNER_ARCH", arch())
        .env("RUNNER_TEMP", temp)
        .env("RUNNER_NAME", "localci")
        .env("GITHUB_OUTPUT", &files.output)
        .env("GITHUB_ENV", &files.env)
        .env("GITHUB_PATH", &files.path)
        .env(
            "GITHUB_STATE",
            files.dir.join(format!("step-{}.state", files.index)),
        )
        .env("GITHUB_STEP_SUMMARY", files.dir.join("summary.md"));
    for (k, v) in env {
        command.env(k, text(v));
    }
    if !extra_path.is_empty() {
        let current = env
            .get("PATH")
            .map(text)
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        command.env("PATH", format!("{}{sep}{current}", extra_path.join(sep)));
    }
    let limit = [
        minutes(step.timeout_minutes.as_ref()).map(|d| Instant::now() + d),
        deadline,
    ]
    .into_iter()
    .flatten()
    .min();
    spawn_teed(command, log, limit, opts.quiet)
}

fn spawn_teed(
    mut command: Command,
    log: &Path,
    limit: Option<Instant>,
    quiet: bool,
) -> Result<(), String> {
    let file = Arc::new(Mutex::new(
        fs::File::create(log).map_err(|e| format!("cannot create log: {e}"))?,
    ));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {:?}: {e}", command.get_program()))?;
    let pumps: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut source| {
        let file = Arc::clone(&file);
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n) = source.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut f) = file.lock() {
                    let _ = f.write_all(&buf[..n]);
                }
                if !quiet {
                    let _ = std::io::stderr().write_all(&buf[..n]);
                }
            }
        })
    })
    .collect();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if limit.is_some_and(|l| Instant::now() >= l) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("timed out (timeout-minutes)".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => break Err(e.to_string()),
        }
    };
    for pump in pumps {
        let _ = pump.join();
    }
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(status
            .code()
            .map_or_else(|| "killed by a signal".into(), |c| format!("exit {c}")))
    }
}

type StepWrites = (Map<String, Value>, Vec<(String, String)>, Vec<String>);

struct StepFiles {
    dir: PathBuf,
    index: usize,
    output: PathBuf,
    env: PathBuf,
    path: PathBuf,
}

impl StepFiles {
    fn new(dir: &Path, index: usize) -> StepFiles {
        let file = |kind: &str| {
            let p = dir.join(format!("step-{index}.{kind}"));
            let _ = fs::write(&p, "");
            p
        };
        StepFiles {
            dir: dir.to_path_buf(),
            index,
            output: file("output"),
            env: file("env"),
            path: file("path"),
        }
    }

    /// The step's outputs, its `GITHUB_ENV` additions, and its `GITHUB_PATH` entries.
    fn read(&self) -> StepWrites {
        let read = |p: &Path| fs::read_to_string(p).unwrap_or_default();
        let outputs = parse_kv(&read(&self.output))
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let path = read(&self.path)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .rev()
            .collect();
        (outputs, parse_kv(&read(&self.env)), path)
    }
}

/// `KEY=value` lines and `KEY<<DELIM` blocks, as GitHub's environment files use.
pub fn parse_kv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_end_matches('\r');
        if let Some((key, delim)) = line.split_once("<<") {
            let mut value = Vec::new();
            for l in lines.by_ref() {
                if l.trim_end_matches('\r') == delim {
                    break;
                }
                value.push(l.trim_end_matches('\r'));
            }
            out.push((key.to_string(), value.join("\n")));
        } else if let Some((key, value)) = line.split_once('=') {
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
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

fn minutes(v: Option<&serde_yaml::Value>) -> Option<Duration> {
    let m = v
        .map(to_json)
        .and_then(|v| v.as_f64().or_else(|| expr::to_text(&v).parse().ok()))?;
    (m > 0.0).then(|| Duration::from_secs_f64(m * 60.0))
}

fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "ARM64",
        "x86" => "X86",
        "arm" => "ARM",
        _ => "X64",
    }
}

fn print_tail(log: &Path, n: usize) {
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines[lines.len().saturating_sub(n)..] {
        eprintln!("   │ {line}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_file_formats() {
        let kv = super::parse_kv("a=1\nmulti<<EOF\nx\ny\nEOF\nb=c=d\n");
        assert_eq!(
            kv,
            vec![
                ("a".into(), "1".into()),
                ("multi".into(), "x\ny".into()),
                ("b".into(), "c=d".into())
            ]
        );
    }
}
