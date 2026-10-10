//! localci: run the GitHub Actions jobs that match this host, locally.

use localci::{corpus, expr, plan, run, skill, workflow};
use plan::Selection;
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use workflow::Os;

const HELP: &str = "localci: run the GitHub Actions jobs that match this host, locally

USAGE:
  localci list [OPTIONS] [TARGET...]   the workflow's legs and which run here
  localci plan [OPTIONS] [TARGET...]   the picked legs and the steps that would run
  localci run  [OPTIONS] [TARGET...]   run them in this checkout
  localci skill install                install the agent skill for Claude Code, Codex and ~/.agents
  localci check [PATH...]              check that localci understands every workflow under PATH
                                       (a repository, a workflow file, or a folder of repositories)

TARGET is a leg key (rust-test#2), a job id (clippy), or text in a leg's name
(\"linux x64 libs\"). No target picks every leg whose runs-on matches this OS.

OPTIONS:
  -w, --workflow FILE   workflow file or name in .github/workflows (default: ci.yml)
      --event NAME      github.event_name (default: pull_request off the default branch, else push)
      --matrix K=V      only legs whose matrix has K=V (repeatable)
      --skip TEXT       leave out steps whose name or command contains TEXT, or LEG:N (repeatable)
      --only TEXT       run only steps matching TEXT or LEG:N (repeatable)
      --skip-setup      leave out steps that install packages or tools
      --ignore-if       run picked jobs even when their job-level if: is false here
      --no-services     do not start the jobs' services: containers (they run already)
      --input K=V       a workflow_dispatch input (repeatable)
  -k, --keep-going      keep running legs after one fails
  -q, --quiet           hide step output; print the tail of a failing step's log
      --json            print the plan or the run report as JSON on stdout
  -V, --version
  -h, --help
";

struct Args {
    command: String,
    workflow: Option<String>,
    event: Option<String>,
    inputs: Map<String, Value>,
    selection: Selection,
    keep_going: bool,
    quiet: bool,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut raw = std::env::args().skip(1);
    let mut args = Args {
        command: String::new(),
        workflow: None,
        event: None,
        inputs: Map::new(),
        selection: Selection::default(),
        keep_going: false,
        quiet: false,
        json: false,
    };
    let pair = |v: String| {
        v.split_once('=')
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .ok_or(format!("expected K=V, got `{v}`"))
    };
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" | "version" => {
                println!("localci {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-w" | "--workflow" => args.workflow = Some(value(&arg)?),
            "--event" => args.event = Some(value(&arg)?),
            "--matrix" => args.selection.matrix.push(pair(value(&arg)?)?),
            "--input" => {
                let (k, v) = pair(value(&arg)?)?;
                args.inputs.insert(k, Value::String(v));
            }
            "--skip" => args.selection.skip.push(value(&arg)?),
            "--only" => args.selection.only.push(value(&arg)?),
            "--skip-setup" => args.selection.skip_setup = true,
            "--ignore-if" => args.selection.ignore_if = true,
            "--no-services" => args.selection.no_services = true,
            "-k" | "--keep-going" => args.keep_going = true,
            "-q" | "--quiet" => args.quiet = true,
            "--json" => args.json = true,
            flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`")),
            word if args.command.is_empty()
                && matches!(word, "list" | "plan" | "run" | "check") =>
            {
                args.command = word.into()
            }
            word => args.selection.targets.push(word.into()),
        }
    }
    if args.command.is_empty() {
        args.command = "plan".into();
    }
    Ok(args)
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn find_workflow(root: &Path, name: Option<&str>) -> Result<PathBuf, String> {
    let dir = root.join(".github/workflows");
    let candidates = |n: &str| {
        [
            PathBuf::from(n),
            dir.join(n),
            dir.join(format!("{n}.yml")),
            dir.join(format!("{n}.yaml")),
        ]
    };
    let name = name.unwrap_or("ci");
    if let Some(found) = candidates(name).into_iter().find(|p| p.is_file()) {
        return Ok(found);
    }
    let available: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    Err(format!(
        "no workflow `{name}` in {} (found: {})",
        dir.display(),
        available.join(", ")
    ))
}

fn base_contexts(root: &Path, args: &Args, workflow_name: &str) -> Map<String, Value> {
    let sha = git(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    let branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"]);
    let default_branch = git(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .and_then(|r| r.strip_prefix("origin/").map(String::from))
    .unwrap_or_else(|| "main".into());
    let repository = git(root, &["remote", "get-url", "origin"])
        .map(|url| {
            let url = url
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .replace(':', "/");
            url.rsplit('/')
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();
    let on_default = branch.as_deref() == Some(default_branch.as_str());
    let event = args
        .event
        .clone()
        .unwrap_or_else(|| if on_default { "push" } else { "pull_request" }.into());
    let head_ref = branch.clone().unwrap_or_else(|| "HEAD".into());
    let base_sha = git(
        root,
        &["merge-base", "HEAD", &format!("origin/{default_branch}")],
    )
    .unwrap_or_default();
    let before = git(root, &["rev-parse", "HEAD~1"]).unwrap_or_default();
    let (git_ref, event_body) = if event == "pull_request" {
        (
            "refs/pull/0/merge".to_string(),
            json!({ "pull_request": {
            "base": { "sha": base_sha, "ref": default_branch }, "head": { "sha": sha, "ref": head_ref } } }),
        )
    } else {
        (
            format!("refs/heads/{head_ref}"),
            json!({ "before": before, "after": sha, "inputs": args.inputs }),
        )
    };
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .unwrap_or_default();
    let process_env: Map<String, Value> = std::env::vars()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let mut contexts = Map::new();
    contexts.insert("github".into(), json!({
        "repository": repository,
        "repository_owner": repository.split('/').next().unwrap_or_default(),
        "ref": git_ref,
        "ref_name": head_ref,
        "head_ref": if event == "pull_request" { head_ref.clone() } else { String::new() },
        "base_ref": if event == "pull_request" { default_branch.clone() } else { String::new() },
        "sha": sha, "workflow_sha": sha,
        "event_name": event, "event": event_body,
        "workflow": workflow_name, "workspace": root, "token": token,
        "actor": std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default(),
        "run_id": "0", "run_number": "0", "run_attempt": "1",
        "server_url": "https://github.com", "api_url": "https://api.github.com",
    }));
    // Locally, secrets and variables come from the environment of the same name.
    contexts.insert("secrets".into(), Value::Object(process_env.clone()));
    // `vars.LOCALCI` and `env.LOCALCI` mark a local run; on GitHub they are empty (ADR-0008).
    let mut vars = process_env;
    vars.insert("LOCALCI".into(), Value::String("true".into()));
    contexts.insert("vars".into(), Value::Object(vars));
    contexts.insert("inputs".into(), Value::Object(args.inputs.clone()));
    contexts.insert("env".into(), Value::Object(Map::new()));
    contexts
}

fn run_dir(root: &Path) -> PathBuf {
    let base = std::env::temp_dir().join("localci");
    if let Ok(entries) = std::fs::read_dir(&base) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for old in dirs.iter().rev().skip(19) {
            let _ = std::fs::remove_dir_all(old);
        }
    }
    let name = root
        .file_name()
        .map_or("repo".into(), |n| n.to_string_lossy().into_owned());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    base.join(format!("{stamp}-{name}"))
}

fn check(root: &Path, paths: &[String]) -> ExitCode {
    let paths: Vec<PathBuf> = if paths.is_empty() {
        vec![root.to_path_buf()]
    } else {
        paths.iter().map(PathBuf::from).collect()
    };
    let files: Vec<PathBuf> = paths
        .iter()
        .flat_map(|p| corpus::workflow_files(p))
        .collect();
    let mut errors = 0;
    for file in &files {
        for finding in corpus::check_file(file) {
            let mark = if finding.level == corpus::Level::Error {
                errors += 1;
                "error"
            } else {
                "note "
            };
            println!("{mark} {}: {}", finding.file.display(), finding.message);
        }
    }
    println!("localci: {} workflow files, {errors} errors", files.len());
    if errors > 0 || files.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("localci: {e}");
            ExitCode::from(2)
        }
    }
}

fn real_main() -> Result<ExitCode, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("skill") {
        return skill::main(&raw[1..]);
    }
    let args = parse_args()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = git(&cwd, &["rev-parse", "--show-toplevel"]).map_or(cwd, PathBuf::from);
    if args.command == "check" {
        return Ok(check(&root, &args.selection.targets));
    }
    let path = find_workflow(&root, args.workflow.as_deref())?;
    let (workflow, jobs) = workflow::load(&path)?;
    let workflow_name = workflow.name.clone().unwrap_or_else(|| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let contexts = base_contexts(&root, &args, &workflow_name);
    let legs = workflow::expand(&jobs, &contexts);
    let host = Os::host();
    let sel = &args.selection;

    if args.command != "run" {
        if args.json {
            let mut plan = plan::to_json(&legs, sel, host, &contexts);
            plan["workflow"] = json!(path);
            plan["event"] = contexts["github"]["event_name"].clone();
            println!(
                "{}",
                serde_json::to_string_pretty(&plan).unwrap_or_default()
            );
        } else if args.command == "list" {
            for leg in &legs {
                let mark = if leg.os == Some(host) {
                    if sel.picks(leg) {
                        "▶"
                    } else {
                        "·"
                    }
                } else {
                    "✗"
                };
                println!(
                    "{mark} {:<22} {:<40} {}",
                    leg.key,
                    leg.name,
                    leg.labels.join(", ")
                );
            }
        } else {
            println!(
                "{} · event {} · host {}\n",
                path.display(),
                expr::to_text(&contexts["github"]["event_name"]),
                host.runner_name()
            );
            plan::print(&legs, sel, host, &contexts);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let picked: Vec<&workflow::Leg> = legs
        .iter()
        .filter(|l| l.os == Some(host) && sel.picks(l))
        .collect();
    if picked.is_empty() {
        let elsewhere: Vec<&str> = legs
            .iter()
            .filter(|l| sel.picks(l))
            .map(|l| l.key.as_str())
            .collect();
        return Err(if elsewhere.is_empty() {
            "nothing matches; `localci list` shows the legs".into()
        } else {
            format!(
                "{} need another OS than {}",
                elsewhere.join(", "),
                host.runner_name()
            )
        });
    }
    let dir = run_dir(&root);
    run::private_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if let Some(warning) = run::temp_warning() {
        eprintln!("localci: warning: {warning}");
    }
    let opts = run::Options {
        workspace: root,
        run_dir: dir.clone(),
        keep_going: args.keep_going,
        quiet: args.quiet,
        host,
    };
    let started = Instant::now();
    let reports = run::run_all(&picked, &workflow, &contexts, sel, &opts);
    let failed = reports.iter().filter(|r| r.result == "failure").count();
    let report = json!({ "workflow": path, "logs": dir, "seconds": started.elapsed().as_secs_f64(), "failed": failed, "legs": reports });
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    let _ = std::fs::write(dir.join("report.json"), &text);

    eprintln!(
        "\nlocalci: {} legs in {:.0}s · logs {}",
        reports.len(),
        started.elapsed().as_secs_f64(),
        dir.display()
    );
    for r in &reports {
        let mark = match r.result {
            "success" => "✓",
            "failure" => "✗",
            _ => "-",
        };
        let detail = match (r.failed_step, &r.reason) {
            (Some(n), _) => format!(
                "  step {n}: {}",
                r.steps
                    .iter()
                    .find(|s| s.index == n)
                    .map_or("", |s| s.name.as_str())
            ),
            (None, Some(reason)) => format!("  {reason}"),
            _ => String::new(),
        };
        eprintln!(
            "  {mark} {:<22} {} ({:.0}s){detail}",
            r.key, r.name, r.seconds
        );
    }
    if args.json {
        println!("{text}");
    }
    Ok(if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
