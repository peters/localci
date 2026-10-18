//! `localci setup`: the toolchain plan of the picked legs, and the install (ADR-0012).

use crate::install::{self, Activation, Lock};
use crate::toolchain::{self, Need};
use crate::workflow::{Leg, Workflow};
use serde_json::{json, Map, Value};
use std::path::Path;

/// The needs of the picked legs. Equal requests from several legs are one install.
pub fn plan(
    legs: &[&Leg],
    workflow: &Workflow,
    workspace: &Path,
    contexts: &Map<String, Value>,
) -> Vec<Need> {
    legs.iter()
        .flat_map(|leg| toolchain::needs_for(leg, workspace, contexts, Some(&workflow.env)))
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

/// Installs every need that localci can install. Returns the report and whether
/// an install failed.
pub fn install_all(
    needs: &[Need],
    dir: &Path,
    workspace: &Path,
    quiet: bool,
) -> Result<(Value, bool), String> {
    let _lock = Lock::take(dir)?;
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
                        json!({ "tool": need.tool, "request": need.request, "version": i.version,
                                "path": i.path, "result": verb })
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
        said.push(format!(
            "{} {} from {}",
            title(need),
            a.version,
            dir.display()
        ));
        activations.push(a);
    }
    Ok((activations, format!("toolchain: {}", said.join("; "))))
}
