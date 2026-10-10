//! `localci check`: whether localci understands every workflow in a set of repositories.
//! It parses each file, expands every matrix, and evaluates every `${{ }}` and `if:`.

use crate::expr::{self, Ctx, Status};
use crate::workflow::{self, yaml_text};
use serde_json::{json, Map, Value};
use serde_yaml::Value as Yaml;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Level {
    /// localci cannot run this workflow correctly.
    Error,
    /// Something that is skipped or needs another host, by design.
    Note,
}

#[derive(Debug)]
pub struct Finding {
    pub file: PathBuf,
    pub level: Level,
    pub message: String,
}

/// Workflow files under `path`: a repository, a workflows directory, a single file, or a
/// directory whose children are repositories.
pub fn workflow_files(path: &Path) -> Vec<PathBuf> {
    let is_workflow = |p: &Path| {
        p.is_file() && matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
    };
    let list = |dir: &Path| -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| is_workflow(p))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    };
    if is_workflow(path) {
        return vec![path.to_path_buf()];
    }
    let own = path.join(".github/workflows");
    if own.is_dir() {
        return list(&own);
    }
    if path.ends_with("workflows") {
        return list(path);
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.join(".github/workflows").is_dir())
                .collect()
        })
        .unwrap_or_default();
    children.sort();
    children
        .iter()
        .flat_map(|c| list(&c.join(".github/workflows")))
        .collect()
}

/// Contexts shaped like a pull request run, so expressions meet the values they expect.
pub fn sample_contexts() -> Map<String, Value> {
    let mut contexts = Map::new();
    contexts.insert("github".into(), json!({
        "repository": "owner/repo", "repository_owner": "owner", "ref": "refs/pull/1/merge",
        "ref_name": "feature", "head_ref": "feature", "base_ref": "main", "sha": "0".repeat(40),
        "event_name": "pull_request", "workflow": "ci", "job": "job", "token": "", "actor": "someone",
        "run_id": "0", "run_number": "0", "run_attempt": "1", "server_url": "https://github.com",
        "event": { "pull_request": { "number": 1, "base": { "sha": "0", "ref": "main" }, "head": { "sha": "1", "ref": "feature" } } },
    }));
    for name in [
        "secrets", "vars", "inputs", "env", "steps", "needs", "matrix", "strategy",
    ] {
        contexts.insert(name.into(), json!({}));
    }
    contexts.insert(
        "runner".into(),
        json!({ "os": "Linux", "arch": "X64", "temp": "/tmp", "name": "localci" }),
    );
    contexts.insert("job".into(), json!({ "status": "success" }));
    contexts
}

pub fn check_file(path: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut say = |level: Level, message: String| {
        findings.push(Finding {
            file: path.to_path_buf(),
            level,
            message,
        })
    };
    let (_, jobs) = match workflow::load(path) {
        Ok(loaded) => loaded,
        Err(e) => {
            say(Level::Error, format!("does not load: {e}"));
            return findings;
        }
    };
    let contexts = sample_contexts();
    for leg in workflow::expand(&jobs, &contexts) {
        if leg.job.uses.is_some() {
            say(
                Level::Note,
                format!(
                    "{}: calls a reusable workflow, which localci does not run",
                    leg.key
                ),
            );
        } else if leg
            .labels
            .iter()
            .any(|l| l.contains("${{") || l.contains(['\n', '[', '{']))
        {
            say(
                Level::Error,
                format!(
                    "{}: runs-on did not resolve to labels: {:?}",
                    leg.key, leg.labels
                ),
            );
        } else if leg.os.is_none() {
            say(
                Level::Note,
                format!(
                    "{}: runs-on {:?} names no Linux, macOS or Windows runner",
                    leg.key, leg.labels
                ),
            );
        }
        if leg.job.container.is_some() {
            say(
                Level::Note,
                format!(
                    "{}: `container:` is ignored; steps run on the host",
                    leg.key
                ),
            );
        }
        for (i, step) in leg.job.steps.iter().enumerate() {
            if step.run.is_none() && step.uses.is_none() {
                say(
                    Level::Error,
                    format!("{} step {}: neither run nor uses", leg.key, i + 1),
                );
            }
        }
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let raw: Yaml = serde_yaml::from_str(&text).unwrap_or(Yaml::Null);
    let ctx = Ctx {
        contexts: &contexts,
        status: Status::Success,
    };
    walk(&raw, "", &mut |at, key, value| {
        let result = if key == "if" {
            expr::condition(&yaml_text(value), &ctx).map(|_| ())
        } else if let Yaml::String(s) = value {
            expr::interpolate(s, &ctx).map(|_| ())
        } else {
            Ok(())
        };
        if let Err(e) = result {
            say(Level::Error, format!("{at}: {e}"));
        }
    });
    findings
}

/// Calls `visit(path, key, value)` for every scalar in the document.
fn walk(value: &Yaml, at: &str, visit: &mut dyn FnMut(&str, &str, &Yaml)) {
    match value {
        Yaml::Mapping(map) => {
            for (k, v) in map {
                let key = yaml_text(k);
                let here = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                if matches!(v, Yaml::Mapping(_) | Yaml::Sequence(_)) {
                    walk(v, &here, visit);
                } else {
                    visit(&here, &key, v);
                }
            }
        }
        Yaml::Sequence(items) => {
            for (i, v) in items.iter().enumerate() {
                let here = format!("{at}[{i}]");
                if matches!(v, Yaml::Mapping(_) | Yaml::Sequence(_)) {
                    walk(v, &here, visit);
                } else {
                    visit(&here, "", v);
                }
            }
        }
        Yaml::Tagged(tagged) => walk(&tagged.value, at, visit),
        _ => {}
    }
}
