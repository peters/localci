//! `localci setup`: the toolchain plan of the picked legs, and the install (ADR-0012).

use crate::install::{self, Activation, Lock};
use crate::plan::Selection;
use crate::toolchain::{self, Need};
use crate::workflow::{Leg, Os, Workflow};
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

/// The needs of the picked legs. Equal requests from several legs are one install.
/// A setup step that `--skip` names, or whose `if:` is false before the run, is not
/// in the plan: `run` would not use it either.
pub fn plan(
    legs: &[&Leg],
    workflow: &Workflow,
    workspace: &Path,
    contexts: &Map<String, Value>,
    sel: &Selection,
) -> Vec<Need> {
    legs.iter()
        .flat_map(|leg| {
            toolchain::needs_for(leg, workspace, contexts, Some(&workflow.env))
                .into_iter()
                .filter(move |n| {
                    let step = &leg.job.steps[n.step - 1];
                    !sel.skips(leg, n.step, step)
                        && crate::plan::static_if(leg, step, contexts) != Some(false)
                })
        })
        .collect()
}

fn same_install(a: &Need, b: &Need) -> bool {
    a.tool == b.tool
        && a.request == b.request
        && a.components == b.components
        && a.targets == b.targets
}

fn status(need: &Need) -> &'static str {
    match need.tool {
        None => "not supported",
        Some(_) if need.request.is_empty() => "nothing to install",
        Some(tool) if tool.installable() => "installs",
        Some(_) => "plan only",
    }
}

fn title(need: &Need) -> String {
    need.tool
        .map_or_else(|| need.action.clone(), |t| t.title().to_string())
}

pub fn print_plan(needs: &[Need], dir: &Path) {
    if needs.is_empty() {
        println!("No setup action in the picked legs declares a toolchain.");
        return;
    }
    println!("Toolchains folder: {}\n", dir.display());
    for n in needs {
        let request = if n.request.is_empty() {
            "-"
        } else {
            n.request.as_str()
        };
        let kind = if n.exact { "exact" } else { "request" };
        let source = if n.source.is_empty() {
            "-"
        } else {
            n.source.as_str()
        };
        println!(
            "{:<22} step {:<2} {:<10} {:<14} {kind:<7} from {:<24} {}",
            n.leg,
            n.step,
            title(n),
            request,
            source,
            status(n)
        );
        for extra in [&n.policy, &n.note].into_iter().flatten() {
            println!("{:<26}{extra}", "");
        }
    }
}

pub fn plan_json(needs: &[Need], dir: &Path, workspace: &Path) -> Value {
    let items: Vec<Value> = needs
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or_default();
            v["status"] = json!(status(n));
            let installed = install::locate(n, dir, workspace).ok().map(|a| a.version);
            v["installed"] = json!(installed);
            v
        })
        .collect();
    json!({ "toolchains": dir, "needs": items })
}

/// The folders for `PATH` and the variables that make a run use the toolchain of
/// `need`, as the answer of `setup --json` gives them. A caller writes no rule
/// for each tool (ADR-0012). `null` when the toolchain is not in the folder.
fn activation_json(need: &Need, dir: &Path, workspace: &Path) -> Value {
    let Ok(a) = install::locate(need, dir, workspace) else {
        return Value::Null;
    };
    let paths: Vec<String> = a.paths.iter().map(|p| p.display().to_string()).collect();
    let env: Map<String, Value> = a
        .env
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    json!({ "paths": paths, "env": env })
}

/// Installs every need that localci can install. Returns the report and whether
/// an install failed.
pub fn install_all(
    needs: &[Need],
    dir: &Path,
    workspace: &Path,
    quiet: bool,
) -> Result<(Value, bool), String> {
    let _lock = Lock::take(dir)?;
    install::clean_leftovers(dir);
    let mut done: Vec<&Need> = Vec::new();
    let mut results = Vec::new();
    let mut failed = false;
    for need in needs {
        if done.iter().any(|d| same_install(d, need)) {
            continue;
        }
        done.push(need);
        let label = format!("{} {}", title(need), need.request);
        let result = match status(need) {
            "installs" => {
                if !quiet {
                    eprintln!("localci: setup {label} ...");
                }
                match install::install(need, dir, workspace) {
                    Ok(i) => {
                        let verb = if i.fresh {
                            "installed"
                        } else {
                            "already there"
                        };
                        eprintln!("  ✓ {label}: {} {verb} ({})", i.version, i.path.display());
                        let mut result = json!({ "tool": need.tool, "request": need.request,
                                "version": i.version, "path": i.path, "result": verb });
                        result["activation"] = activation_json(need, dir, workspace);
                        result
                    }
                    Err(why) => {
                        failed = true;
                        eprintln!("  ✗ {label}: {why}");
                        json!({ "tool": need.tool, "request": need.request, "result": "failed", "error": why })
                    }
                }
            }
            other => {
                let why = need.note.clone().unwrap_or_else(|| other.to_string());
                eprintln!("  - {label}: not installed ({why})");
                json!({ "tool": need.tool, "action": need.action, "request": need.request,
                        "result": "not installed", "reason": why })
            }
        };
        results.push(result);
    }
    Ok((json!({ "toolchains": dir, "results": results }), failed))
}

/// What a run does at a setup step when `--toolchains` is given: the toolchains of
/// the step from the folder, or the reason why the step does nothing.
pub fn activate_step(
    leg: &Leg,
    index: usize,
    workspace: &Path,
    contexts: &Map<String, Value>,
    dir: &Path,
) -> Result<(Vec<Activation>, String), String> {
    let needs: Vec<Need> = toolchain::needs_for(leg, workspace, contexts, None)
        .into_iter()
        .filter(|n| n.step == index)
        .collect();
    let mut activations = Vec::new();
    let mut said = Vec::new();
    for need in &needs {
        if need.tool.is_some() && need.request.is_empty() {
            said.push(need.note.clone().unwrap_or_default());
            continue;
        }
        let a = install::locate(need, dir, workspace)?;
        let line = format!("{} {} from {}", title(need), a.version, dir.display());
        if !said.contains(&line) {
            said.push(line);
        }
        activations.push(a);
    }
    Ok((activations, format!("toolchain: {}", said.join("; "))))
}

/// The options of `localci setup`.
pub struct Flags {
    pub toolchains: Option<PathBuf>,
    pub plan_only: bool,
    pub json: bool,
    pub quiet: bool,
}

/// `PATH`, `DOTNET_ROOT` and `RUSTUP_HOME` need an absolute folder: a step with a
/// `working-directory` would read a relative one from another place.
pub fn absolute(dir: &Path) -> PathBuf {
    std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// `localci setup [--plan]`: the toolchains of the picked legs (ADR-0012).
pub fn command(
    legs: &[Leg],
    workflow: &Workflow,
    root: &Path,
    contexts: &Map<String, Value>,
    sel: &Selection,
    flags: &Flags,
) -> Result<ExitCode, String> {
    let host = Os::host();
    let picked: Vec<&Leg> = legs
        .iter()
        .filter(|l| l.os == Some(host) && sel.picks(l))
        .collect();
    let dir = match flags
        .toolchains
        .as_ref()
        .filter(|d| !d.as_os_str().is_empty())
    {
        Some(dir) => dir.clone(),
        None => install::default_dir().ok_or("no toolchains folder: pass --toolchains DIR")?,
    };
    let dir = absolute(&dir);
    let needs = plan(&picked, workflow, root, contexts, sel);
    if flags.plan_only {
        if flags.json {
            let plan = plan_json(&needs, &dir, root);
            println!(
                "{}",
                serde_json::to_string_pretty(&plan).unwrap_or_default()
            );
        } else {
            print_plan(&needs, &dir);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let (report, failed) = install_all(&needs, &dir, root, flags.quiet)?;
    if flags.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    }
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::Tool;

    #[test]
    fn the_answer_gives_the_activation_of_each_install() {
        let dir = std::env::temp_dir().join(format!("localci-setup-act-{}", install::unique()));
        let bin = dir.join("go").join("1.23.4").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("go"), "#!/bin/sh\n").unwrap();
        let need = |tool: Tool, action: &str, request: &str, exact: bool| Need {
            leg: "build".into(),
            step: 1,
            action: action.into(),
            tool: Some(tool),
            request: request.into(),
            source: "with.go-version".into(),
            exact,
            policy: None,
            components: vec![],
            targets: vec![],
            note: None,
            env: Default::default(),
        };
        // Go 1.23.4 is in the folder, so the install needs no network. Python is in
        // the plan only, so setup does not install it.
        let needs = [
            need(Tool::Go, "actions/setup-go@v5", "1.23.4", true),
            need(Tool::Python, "actions/setup-python@v5", "3.12", false),
        ];
        let (report, failed) = install_all(&needs, &dir, &dir, true).unwrap();
        assert!(!failed);
        let results = report["results"].as_array().unwrap();
        assert_eq!(results[0]["version"], "1.23.4");
        assert_eq!(results[0]["result"], "already there");
        assert_eq!(
            results[0]["activation"],
            json!({
                "paths": [bin.display().to_string()],
                "env": { "GOROOT": dir.join("go").join("1.23.4").display().to_string() }
            })
        );
        assert_eq!(results[1]["result"], "not installed");
        assert!(results[1].get("activation").is_none(), "{}", results[1]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
