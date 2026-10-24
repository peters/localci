//! The toolchains that the setup actions of a leg declare (ADR-0012).
//!
//! localci reads the `with:` inputs of known setup actions and the version files
//! that they point at. It does not guess: a range that only the action can resolve
//! is reported as such, with the text of the request.

use crate::expr::{self, Ctx, Status};
use crate::workflow::{yaml_text, Leg};
use serde::Serialize;
use serde_json::{Map, Value};
use serde_yaml::Value as Yaml;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Dotnet,
    Node,
    Rust,
    Go,
    Python,
    Java,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Dotnet => "dotnet",
            Tool::Node => "node",
            Tool::Rust => "rust",
            Tool::Go => "go",
            Tool::Python => "python",
            Tool::Java => "java",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tool::Dotnet => ".NET SDK",
            Tool::Node => "Node.js",
            Tool::Rust => "Rust",
            Tool::Go => "Go",
            Tool::Python => "Python",
            Tool::Java => "Java",
        }
    }

    /// `localci setup` installs these. Python is in the plan only.
    pub fn installable(self) -> bool {
        matches!(
            self,
            Tool::Dotnet | Tool::Node | Tool::Rust | Tool::Go | Tool::Java
        )
    }
}

/// One toolchain that one setup step asks for.
#[derive(Clone, Debug, Serialize)]
pub struct Need {
    pub leg: String,
    pub step: usize,
    /// The `uses:` reference, for example `actions/setup-node@v4`.
    pub action: String,
    /// `None` for a setup action that localci does not know.
    pub tool: Option<Tool>,
    /// The version text as the workflow or the version file gives it.
    pub request: String,
    /// Where the request comes from: an input (`with.node-version`) or a file.
    pub source: String,
    /// True when the request names one exact version.
    pub exact: bool,
    /// A rule that the tool applies itself, for example the `rollForward` of `global.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Variables that the action sets for later steps, for example `RUSTFLAGS`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// The action name without the owner's version ref, in lower case.
fn action_name(uses: &str) -> String {
    uses.split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

const KNOWN: &[(&str, Tool)] = &[
    ("actions/setup-dotnet", Tool::Dotnet),
    ("actions/setup-node", Tool::Node),
    ("dtolnay/rust-toolchain", Tool::Rust),
    ("actions-rust-lang/setup-rust-toolchain", Tool::Rust),
    ("actions/setup-go", Tool::Go),
    ("actions/setup-python", Tool::Python),
    ("actions/setup-java", Tool::Java),
];

/// The tool of a known setup action.
pub fn tool_of(uses: &str) -> Option<Tool> {
    let name = action_name(uses);
    KNOWN.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// An action that sets up a toolchain, known or not: `*/setup-*` or a known name.
pub fn is_setup_action(uses: &str) -> bool {
    let name = action_name(uses);
    tool_of(uses).is_some()
        || name
            .rsplit('/')
            .next()
            .is_some_and(|n| n.starts_with("setup-") || n.ends_with("-toolchain"))
}

/// Evaluates `source` into the `env` context, as a run does for workflow, job and step env.
/// Adds to `later` the names whose value comes from a step or a job during the run.
fn add_env(
    source: &BTreeMap<String, Yaml>,
    contexts: &mut Map<String, Value>,
    later: &mut Vec<String>,
) {
    for (k, v) in source {
        let ctx = Ctx {
            contexts,
            status: Status::Success,
        };
        let raw = yaml_text(v);
        if raw.contains("steps.") || raw.contains("needs.") {
            later.push(k.clone());
        }
        let value = expr::interpolate(&raw, &ctx).unwrap_or(raw);
        if let Some(Value::Object(env)) = contexts.get_mut("env") {
            env.insert(k.clone(), Value::String(value));
        }
    }
}

/// The toolchains that the setup steps of `leg` ask for. Before a run, pass the
/// workflow `env:`; during a run, `base` has the env and the steps already.
pub fn needs_for(
    leg: &Leg,
    workspace: &Path,
    base: &Map<String, Value>,
    workflow_env: Option<&BTreeMap<String, Yaml>>,
) -> Vec<Need> {
    let mut contexts = base.clone();
    contexts.insert("matrix".into(), Value::Object(leg.matrix.clone()));
    let mut dynamic = Vec::new();
    if let Some(workflow_env) = workflow_env {
        contexts.insert("env".into(), Value::Object(Map::new()));
        add_env(workflow_env, &mut contexts, &mut dynamic);
        add_env(&leg.job.env, &mut contexts, &mut dynamic);
    }
    let mut out = Vec::new();
    for (i, step) in leg.job.steps.iter().enumerate() {
        let Some(uses) = step.uses.as_deref() else {
            continue;
        };
        if !is_setup_action(uses) {
            continue;
        }
        let mut step_contexts = contexts.clone();
        let mut step_dynamic = dynamic.clone();
        add_env(&step.env, &mut step_contexts, &mut step_dynamic);
        let ctx = Ctx {
            contexts: &step_contexts,
            status: Status::Success,
        };
        // Before a run, an input from an earlier step or job is not known yet. The
        // action does not fall back to a version file then, so the plan does not either.
        let from_run = |raw: &str| {
            raw.contains("steps.")
                || raw.contains("needs.")
                || step_dynamic
                    .iter()
                    .any(|n| raw.contains(&format!("env.{n}")))
        };
        // Only the inputs that name a version or its file decide this.
        let later = workflow_env.is_some()
            && step
                .with
                .iter()
                .filter(|(k, _)| VERSION_INPUTS.contains(&k.as_str()))
                .any(|(_, v)| from_run(&yaml_text(v)));
        let input = |name: &str| -> Option<String> {
            let raw = yaml_text(step.with.get(name)?);
            let text = expr::interpolate(&raw, &ctx).unwrap_or(raw);
            let text = text.trim().to_string();
            // An empty `rustflags` turns the action's default flags off.
            (!text.is_empty() || name == "rustflags").then_some(text)
        };
        let base_need = Need {
            leg: leg.key.clone(),
            step: i + 1,
            action: uses.to_string(),
            tool: tool_of(uses),
            request: String::new(),
            source: String::new(),
            exact: false,
            policy: None,
            components: Vec::new(),
            targets: Vec::new(),
            note: None,
            env: BTreeMap::new(),
        };
        let found = match tool_of(uses) {
            None => vec![Need {
                note: Some("this setup action is not supported".into()),
                ..base_need
            }],
            Some(_) if later => vec![Need {
                note: Some("an earlier step or job sets the version during the run".into()),
                ..base_need
            }],
            Some(tool) => crate::toolchain_inputs::read(tool, uses, &input, workspace, base_need),
        };
        out.extend(found);
    }
    out
}

/// The inputs of the known actions that name a version or the file of a version.
const VERSION_INPUTS: &[&str] = &[
    "dotnet-version",
    "global-json-file",
    "node-version",
    "node-version-file",
    "toolchain",
    "go-version",
    "go-version-file",
    "python-version",
    "python-version-file",
    "java-version",
    "java-version-file",
    "distribution",
];

/// `true` for a version with three numeric parts, `v` allowed: `20.11.1`, `v8.0.403`.
pub fn is_exact(version: &str) -> bool {
    let v = version.trim().trim_start_matches('v');
    let parts: Vec<&str> = v
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}
