//! The inputs and version files of each known setup action (ADR-0012).

use crate::toolchain::{is_exact, Need, Tool};
use serde_json::Value;
use std::path::Path;

/// The needs of one setup step of a known action.
pub fn read(
    tool: Tool,
    uses: &str,
    input: &dyn Fn(&str) -> Option<String>,
    workspace: &Path,
    base: Need,
) -> Vec<Need> {
    match tool {
        Tool::Dotnet => dotnet(input, workspace, base),
        Tool::Node => node(input, workspace, base),
        Tool::Rust => rust(uses, input, workspace, base),
        Tool::Go => simple(
            input,
            workspace,
            base,
            "go-version",
            "go-version-file",
            go_file,
        ),
        Tool::Python => simple(
            input,
            workspace,
            base,
            "python-version",
            "python-version-file",
            first_line,
        ),
        Tool::Java => java(input, workspace, base),
    }
}

fn need(base: &Need, request: &str, source: &str) -> Need {
    Need {
        request: request.trim().to_string(),
        source: source.to_string(),
        exact: is_exact(request),
        ..base.clone()
    }
}

fn missing(base: Need, note: &str) -> Vec<Need> {
    vec![Need {
        note: Some(note.into()),
        ..base
    }]
}

fn read_file(workspace: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(workspace.join(name)).ok()
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
}

/// `actions/setup-dotnet`: `dotnet-version` (one version for each line), else
/// `global-json-file`, else the `global.json` of the repository.
fn dotnet(input: &dyn Fn(&str) -> Option<String>, workspace: &Path, base: Need) -> Vec<Need> {
    if let Some(versions) = input("dotnet-version") {
        return versions
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|v| need(&base, v, "with.dotnet-version"))
            .collect();
    }
    let file = input("global-json-file").unwrap_or_else(|| "global.json".into());
    let Some(text) = read_file(workspace, &file) else {
        return missing(
            base,
            "no dotnet-version and no global.json: the runner's SDK is used",
        );
    };
    let json: Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => return missing(base, &format!("{file}: {e}")),
    };
    let Some(version) = json["sdk"]["version"].as_str() else {
        return missing(base, &format!("{file} has no sdk.version"));
    };
    let roll = json["sdk"]["rollForward"].as_str().unwrap_or("latestPatch");
    vec![Need {
        policy: Some(format!("rollForward: {roll}")),
        ..need(&base, version, &file)
    }]
}

/// `actions/setup-node`: `node-version`, else `node-version-file`. Without both,
/// the action uses the node of the runner.
fn node(input: &dyn Fn(&str) -> Option<String>, workspace: &Path, base: Need) -> Vec<Need> {
    if let Some(version) = input("node-version") {
        return vec![need(&base, &version, "with.node-version")];
    }
    let Some(file) = input("node-version-file") else {
        return missing(base, "no node-version: the runner's node is used");
    };
    let Some(text) = read_file(workspace, &file) else {
        return missing(base, &format!("{file} is missing"));
    };
    let name = Path::new(&file)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let version = if name == "package.json" {
        // The action reads volta.node, then engines.node.
        serde_json::from_str::<Value>(&text).ok().and_then(|json| {
            json["volta"]["node"]
                .as_str()
                .or(json["engines"]["node"].as_str())
                .map(String::from)
        })
    } else if name == ".tool-versions" {
        text.lines().find_map(|l| {
            l.trim()
                .strip_prefix("nodejs ")
                .map(|v| v.trim().to_string())
        })
    } else {
        first_line(&text)
    };
    match version {
        Some(v) => vec![need(&base, &v, &file)],
        None => missing(base, &format!("{file} names no node version")),
    }
}

/// The channel, components and targets of `rust-toolchain.toml` or `rust-toolchain`.
fn toolchain_file(workspace: &Path) -> Option<(String, String, Vec<String>, Vec<String>)> {
    for name in ["rust-toolchain.toml", "rust-toolchain"] {
        let Some(text) = read_file(workspace, name) else {
            continue;
        };
        if !text.contains('=') {
            return first_line(&text).map(|c| (c, name.to_string(), vec![], vec![]));
        }
        let mut channel = None;
        let (mut components, mut targets) = (Vec::new(), Vec::new());
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let list = || -> Vec<String> {
                value
                    .trim()
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            match key.trim() {
                "channel" => channel = Some(value.trim().trim_matches('"').to_string()),
                "components" => components = list(),
                "targets" => targets = list(),
                _ => {}
            }
        }
        return channel.map(|c| (c, name.to_string(), components, targets));
    }
    None
}

fn split_list(text: Option<String>) -> Vec<String> {
    text.map(|t| {
        t.split([',', '\n', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

/// `dtolnay/rust-toolchain@<toolchain>` with an optional `toolchain` input, and
/// `actions-rust-lang/setup-rust-toolchain`, which reads the toolchain file first.
fn rust(
    uses: &str,
    input: &dyn Fn(&str) -> Option<String>,
    workspace: &Path,
    base: Need,
) -> Vec<Need> {
    let components = split_list(input("components"));
    let targets = split_list(input("targets").or_else(|| input("target")));
    let with = |request: String, source: String, extra: (Vec<String>, Vec<String>)| Need {
        components: [components.clone(), extra.0].concat(),
        targets: [targets.clone(), extra.1].concat(),
        exact: is_exact(&request) || is_dated(&request),
        request,
        source,
        ..base.clone()
    };
    if let Some(toolchain) = input("toolchain") {
        return vec![with(toolchain, "with.toolchain".into(), (vec![], vec![]))];
    }
    if uses.to_ascii_lowercase().starts_with("dtolnay/") {
        let reference = uses.split_once('@').map(|(_, r)| r).unwrap_or_default();
        if reference.is_empty() || reference == "master" || reference.starts_with("v") {
            return missing(
                base,
                "dtolnay/rust-toolchain needs a toolchain input with this ref",
            );
        }
        return vec![with(
            reference.to_string(),
            "the action ref".into(),
            (vec![], vec![]),
        )];
    }
    match toolchain_file(workspace) {
        Some((channel, file, c, t)) => vec![with(channel, file, (c, t))],
        None => vec![with(
            "stable".into(),
            "the action default".into(),
            (vec![], vec![]),
        )],
    }
}

/// `nightly-2024-05-01`, `1.80.0-x86_64-unknown-linux-gnu`: names one build.
fn is_dated(request: &str) -> bool {
    request
        .rsplit('-')
        .take(3)
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && request.matches('-').count() >= 3
}

/// The `go` line of `go.mod` (or `toolchain go1.x.y`, which wins), or the first line.
fn go_file(text: &str) -> Option<String> {
    let mut go = None;
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("toolchain go") {
            return Some(v.trim().to_string());
        }
        if let Some(v) = line.strip_prefix("go ") {
            go = Some(v.trim().to_string());
        }
    }
    go.or_else(|| first_line(text))
}

fn simple(
    input: &dyn Fn(&str) -> Option<String>,
    workspace: &Path,
    base: Need,
    version_input: &str,
    file_input: &str,
    parse: fn(&str) -> Option<String>,
) -> Vec<Need> {
    if let Some(versions) = input(version_input) {
        return versions
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|v| need(&base, v, &format!("with.{version_input}")))
            .collect();
    }
    let Some(file) = input(file_input) else {
        return missing(
            base,
            &format!("no {version_input}: the runner's version is used"),
        );
    };
    match read_file(workspace, &file).as_deref().and_then(parse) {
        Some(v) => vec![need(&base, &v, &file)],
        None => missing(base, &format!("{file} names no version")),
    }
}

/// `actions/setup-java`: `distribution` and `java-version` (or `java-version-file`).
fn java(input: &dyn Fn(&str) -> Option<String>, workspace: &Path, base: Need) -> Vec<Need> {
    let distribution = input("distribution").unwrap_or_else(|| "(none)".into());
    let mut needs = simple(
        input,
        workspace,
        base,
        "java-version",
        "java-version-file",
        |t| {
            t.lines().find_map(|l| {
                let l = l.trim();
                l.strip_prefix("java ")
                    .map(|v| v.trim().to_string())
                    .or_else(|| (!l.is_empty() && !l.starts_with('#')).then(|| l.to_string()))
            })
        },
    );
    for n in &mut needs {
        n.policy = Some(format!("distribution: {distribution}"));
    }
    needs
}
