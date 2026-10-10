//! Running one step's process: the shell command, its environment, the output that
//! goes to the log and the terminal, and the `GITHUB_*` files it writes.

use crate::expr::{self, Ctx, Status};
use crate::run::Options;
use crate::workflow::{to_json, Leg, Workflow};
use serde_json::{Map, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
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

pub(crate) type StepWrites = (Map<String, Value>, Vec<(String, String)>, Vec<String>);

pub(crate) struct StepFiles {
    dir: PathBuf,
    index: usize,
    output: PathBuf,
    env: PathBuf,
    path: PathBuf,
}

impl StepFiles {
    pub(crate) fn new(dir: &Path, index: usize) -> StepFiles {
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
    pub(crate) fn read(&self) -> StepWrites {
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

pub(crate) fn minutes(v: Option<&serde_yaml::Value>) -> Option<Duration> {
    let m = v
        .map(to_json)
        .and_then(|v| v.as_f64().or_else(|| expr::to_text(&v).parse().ok()))?;
    (m > 0.0).then(|| Duration::from_secs_f64(m * 60.0))
}

pub(crate) fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "ARM64",
        "x86" => "X86",
        "arm" => "ARM",
        _ => "X64",
    }
}

pub(crate) fn print_tail(log: &Path, n: usize) {
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
