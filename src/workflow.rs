//! Loading a workflow file and expanding its jobs into the matrix legs that run.

use crate::expr::{self, Ctx, Status};
use serde::Deserialize;
use serde_json::{Map, Value};
use serde_yaml::Value as Yaml;
use std::{collections::BTreeMap, path::Path};

/// A YAML scalar as text: GitHub reads `run: true` and `name: 3` as strings.
fn text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Yaml::deserialize(d)?;
    Ok((!value.is_null()).then(|| yaml_text(&value)))
}

fn texts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    let map = BTreeMap::<String, Yaml>::deserialize(d)?;
    Ok(map.into_iter().map(|(k, v)| (k, yaml_text(&v))).collect())
}

#[derive(Deserialize, Default, Clone)]
pub struct Defaults {
    #[serde(default)]
    pub run: RunDefaults,
}

#[derive(Deserialize, Default, Clone)]
pub struct RunDefaults {
    #[serde(default, deserialize_with = "text")]
    pub shell: Option<String>,
    #[serde(rename = "working-directory")]
    #[serde(default, deserialize_with = "text")]
    pub working_directory: Option<String>,
}

#[derive(Deserialize)]
pub struct Workflow {
    #[serde(default, deserialize_with = "text")]
    pub name: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Yaml>,
    #[serde(default)]
    pub defaults: Defaults,
    pub jobs: serde_yaml::Mapping,
}

#[derive(Deserialize)]
pub struct Job {
    #[serde(skip)]
    pub id: String,
    #[serde(default, deserialize_with = "text")]
    pub name: Option<String>,
    #[serde(rename = "runs-on")]
    pub runs_on: Option<Yaml>,
    pub strategy: Option<Strategy>,
    #[serde(default)]
    pub needs: Needs,
    #[serde(rename = "if")]
    pub condition: Option<Yaml>,
    #[serde(default)]
    pub env: BTreeMap<String, Yaml>,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    #[serde(deserialize_with = "texts")]
    pub outputs: BTreeMap<String, String>,
    #[serde(rename = "timeout-minutes")]
    pub timeout_minutes: Option<Yaml>,
    pub container: Option<Yaml>,
    pub services: Option<Yaml>,
    /// A reusable workflow call, which runs nothing here.
    #[serde(default, deserialize_with = "text")]
    pub uses: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
pub enum Needs {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl Needs {
    pub fn list(&self) -> Vec<String> {
        match self {
            Needs::None => Vec::new(),
            Needs::One(one) => vec![one.clone()],
            Needs::Many(many) => many.clone(),
        }
    }
}

#[derive(Deserialize)]
pub struct Strategy {
    pub matrix: Option<Yaml>,
}

#[derive(Deserialize, Clone)]
pub struct Step {
    #[serde(default, deserialize_with = "text")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "text")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "text")]
    pub run: Option<String>,
    #[serde(default, deserialize_with = "text")]
    pub uses: Option<String>,
    #[serde(default, deserialize_with = "text")]
    pub shell: Option<String>,
    #[serde(rename = "working-directory")]
    #[serde(default, deserialize_with = "text")]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Yaml>,
    #[serde(rename = "if")]
    pub condition: Option<Yaml>,
    #[serde(rename = "continue-on-error")]
    pub continue_on_error: Option<Yaml>,
    #[serde(rename = "timeout-minutes")]
    pub timeout_minutes: Option<Yaml>,
}

impl Step {
    pub fn label(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        match (&self.uses, &self.run) {
            (Some(uses), _) => format!("Run {uses}"),
            (_, Some(run)) => format!("Run {}", run.lines().next().unwrap_or_default().trim()),
            _ => "(empty step)".into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    pub fn host() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    /// The `runner.os` value.
    pub fn runner_name(self) -> &'static str {
        match self {
            Os::Linux => "Linux",
            Os::Macos => "macOS",
            Os::Windows => "Windows",
        }
    }

    pub fn from_label(label: &str) -> Option<Os> {
        let l = label.to_ascii_lowercase();
        if l.starts_with("ubuntu") || l == "linux" {
            Some(Os::Linux)
        } else if l.starts_with("macos") {
            Some(Os::Macos)
        } else if l.starts_with("windows") {
            Some(Os::Windows)
        } else {
            None
        }
    }
}

/// One job with one matrix combination: what GitHub shows as one check.
pub struct Leg<'w> {
    pub job: &'w Job,
    /// `job`, or `job#2` for the second matrix combination.
    pub key: String,
    pub name: String,
    pub matrix: Map<String, Value>,
    pub labels: Vec<String>,
    pub os: Option<Os>,
}

pub fn load(path: &Path) -> Result<(Workflow, Vec<Job>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let workflow: Workflow =
        serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut jobs = Vec::new();
    for (id, body) in &workflow.jobs {
        let id = id.as_str().ok_or("job ids must be strings")?.to_string();
        let mut job: Job =
            serde_yaml::from_value(body.clone()).map_err(|e| format!("job `{id}`: {e}"))?;
        job.id = id;
        jobs.push(job);
    }
    Ok((workflow, jobs))
}

pub fn to_json(y: &Yaml) -> Value {
    serde_json::to_value(y).unwrap_or(Value::Null)
}

/// Yaml scalars as the text GitHub would see (`true`, `3`, `x`).
pub fn yaml_text(y: &Yaml) -> String {
    expr::to_text(&to_json(y))
}

pub fn expand<'w>(jobs: &'w [Job], base: &Map<String, Value>) -> Vec<Leg<'w>> {
    let mut legs = Vec::new();
    for job in jobs {
        let combos = job
            .strategy
            .as_ref()
            .and_then(|s| s.matrix.as_ref())
            .map_or_else(|| vec![Map::new()], |m| matrix(&to_json(m)));
        let many = combos.len() > 1;
        for (i, combo) in combos.into_iter().enumerate() {
            let mut contexts = base.clone();
            contexts.insert("matrix".into(), Value::Object(combo.clone()));
            let ctx = Ctx {
                contexts: &contexts,
                status: Status::Success,
            };
            let text = |s: &str| expr::interpolate(s, &ctx).unwrap_or_else(|_| s.to_string());
            let name = match &job.name {
                Some(name) => text(name),
                None if many => {
                    let values: Vec<String> = combo.values().map(expr::to_text).collect();
                    format!("{} ({})", job.id, values.join(", "))
                }
                None => job.id.clone(),
            };
            let labels = job
                .runs_on
                .as_ref()
                .map_or_else(Vec::new, |r| runner_labels(&to_json(r), &ctx));
            let os = labels.iter().find_map(|l| Os::from_label(l));
            let key = if many {
                format!("{}#{}", job.id, i + 1)
            } else {
                job.id.clone()
            };
            legs.push(Leg {
                job,
                key,
                name,
                matrix: combo,
                labels,
                os,
            });
        }
    }
    legs
}

/// `runs-on` as labels. A lone `${{ }}` may yield a list, as `${{ matrix.runs-on }}` does.
pub fn runner_labels(value: &Value, ctx: &Ctx) -> Vec<String> {
    match value {
        Value::String(s) => {
            let t = s.trim();
            let lone = t
                .strip_prefix("${{")
                .and_then(|r| r.strip_suffix("}}"))
                .filter(|inner| !inner.contains("${{"));
            match lone.map(|inner| expr::eval(inner, ctx)) {
                Some(Ok(v @ (Value::Array(_) | Value::Object(_)))) => runner_labels(&v, ctx),
                _ => vec![expr::interpolate(s, ctx).unwrap_or_else(|_| s.clone())],
            }
        }
        Value::Array(items) => items.iter().flat_map(|v| runner_labels(v, ctx)).collect(),
        Value::Object(o) => o
            .get("labels")
            .map_or_else(Vec::new, |v| runner_labels(v, ctx)),
        Value::Null => Vec::new(),
        other => vec![expr::to_text(other)],
    }
}

/// Expands `strategy.matrix` with GitHub's include and exclude rules.
pub fn matrix(m: &Value) -> Vec<Map<String, Value>> {
    let Some(m) = m.as_object() else {
        return vec![Map::new()];
    };
    let mut axes: Vec<(&String, Vec<Value>)> = Vec::new();
    for (k, v) in m {
        if k == "include" || k == "exclude" {
            continue;
        }
        axes.push((k, v.as_array().cloned().unwrap_or_else(|| vec![v.clone()])));
    }
    let mut combos = vec![Map::new()];
    if axes.is_empty() {
        combos.clear();
    }
    for (key, values) in &axes {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    c.insert((*key).clone(), v.clone());
                    c
                })
            })
            .collect();
    }
    let matches = |combo: &Map<String, Value>, entry: &Map<String, Value>, only_axes: bool| {
        entry.iter().all(|(k, v)| {
            (only_axes && !axes.iter().any(|(a, _)| *a == k)) || combo.get(k) == Some(v)
        })
    };
    if let Some(Value::Array(excludes)) = m.get("exclude") {
        for ex in excludes.iter().filter_map(Value::as_object) {
            combos.retain(|c| !matches(c, ex, false));
        }
    }
    let original = combos.len();
    if let Some(Value::Array(includes)) = m.get("include") {
        for inc in includes.iter().filter_map(Value::as_object) {
            let mut matched = false;
            for combo in combos.iter_mut().take(original) {
                if matches(combo, inc, true) {
                    for (k, v) in inc {
                        combo.insert(k.clone(), v.clone());
                    }
                    matched = true;
                }
            }
            if !matched {
                combos.push(inc.clone());
            }
        }
    }
    if combos.is_empty() {
        combos.push(Map::new());
    }
    combos
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runs_on_from_a_matrix_list() {
        let contexts: Map<String, Value> =
            json!({"matrix": {"runs-on": ["self-hosted", "ubuntu-large"], "os": "macos-15"}})
                .as_object()
                .unwrap()
                .clone();
        let ctx = Ctx {
            contexts: &contexts,
            status: Status::Success,
        };
        assert_eq!(
            runner_labels(&json!("${{ matrix.runs-on }}"), &ctx),
            ["self-hosted", "ubuntu-large"]
        );
        assert_eq!(
            runner_labels(&json!(["${{ matrix.os }}", "arm64"]), &ctx),
            ["macos-15", "arm64"]
        );
        assert_eq!(
            runner_labels(&json!({"group": "big", "labels": "linux"}), &ctx),
            ["linux"]
        );
        assert_eq!(Os::from_label("ubuntu-large"), Some(Os::Linux));
    }

    #[test]
    fn include_only_matrix() {
        let m = json!({"include": [{"os": "ubuntu-latest", "shard": "ui"}, {"os": "macos-latest", "shard": "ui"}]});
        let combos = matrix(&m);
        assert_eq!(combos.len(), 2);
        assert_eq!(combos[1]["os"], json!("macos-latest"));
    }

    #[test]
    fn axes_include_exclude() {
        let m = json!({"os": ["a", "b"], "v": [1, 2],
                       "exclude": [{"os": "b", "v": 2}],
                       "include": [{"os": "a", "extra": true}, {"os": "c", "v": 9}]});
        let combos = matrix(&m);
        assert_eq!(combos.len(), 4);
        assert_eq!(combos[0]["extra"], json!(true));
        assert_eq!(combos[1]["extra"], json!(true));
        assert!(combos[2].get("extra").is_none());
        assert_eq!(combos[3]["os"], json!("c"));
    }
}
