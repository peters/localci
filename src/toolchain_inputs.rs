//! The inputs and version files of each known setup action (ADR-0012).

use crate::toolchain::{is_exact, Need, Tool};
use serde_json::Value;
use std::path::{Component, Path};

type Input<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The needs of one setup step of a known action.
pub fn read(tool: Tool, uses: &str, input: Input, workspace: &Path, base: Need) -> Vec<Need> {
    match tool {
        Tool::Dotnet => dotnet(input, workspace, base),
        Tool::Node => node(input, workspace, base),
        Tool::Rust => rust(uses, input, workspace, base),
        Tool::Go => simple(input, workspace, base, "go", go_file),
        Tool::Python => simple(input, workspace, base, "python", first_line),
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

/// A file of the checkout. A path outside it (absolute, or with `..`) is not read,
/// so a plan never shows a file of the machine.
fn read_file(workspace: &Path, name: &str) -> Result<String, String> {
    let path = Path::new(name);
    let inside = path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !inside {
        return Err(format!(
            "{name} is outside the checkout; localci does not read it"
        ));
    }
    let full = workspace.join(path);
    // A symlink must not lead outside the checkout either.
    let inside = match (full.canonicalize(), workspace.canonicalize()) {
        (Ok(real), Ok(root)) => real.starts_with(root),
        _ => return Err(format!("{name} is missing")),
    };
    if !inside {
        return Err(format!(
            "{name} leads outside the checkout; localci does not read it"
        ));
    }
    std::fs::read_to_string(&full).map_err(|_| format!("{name} is missing"))
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
}

/// Removes `//` and `/* */` comments outside strings, as the .NET SDK allows them
/// in `global.json`.
fn without_comments(text: &str) -> String {
    let (mut out, mut chars) = (String::new(), text.chars().peekable());
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut last = ' ';
            for c in chars.by_ref() {
                if last == '*' && c == '/' {
                    break;
                }
                last = c;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `actions/setup-dotnet`: each line of `dotnet-version`, and the SDK of
/// `global-json-file`; without both, the `global.json` of the checkout root.
fn dotnet(input: Input, workspace: &Path, base: Need) -> Vec<Need> {
    let mut needs: Vec<Need> = input("dotnet-version")
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|v| need(&base, v, "with.dotnet-version"))
        .collect();
    let file = match input("global-json-file") {
        Some(file) => file,
        None if !needs.is_empty() => return needs,
        None => {
            if read_file(workspace, "global.json").is_err() {
                return missing(
                    base,
                    "no dotnet-version and no global.json: the runner's SDK is used",
                );
            }
            "global.json".into()
        }
    };
    let parsed = read_file(workspace, &file).and_then(|text| {
        let text = text.trim_start_matches('\u{feff}');
        serde_json::from_str::<Value>(&without_comments(text)).map_err(|e| format!("{file}: {e}"))
    });
    match parsed {
        Ok(json) => match json["sdk"]["version"].as_str() {
            Some(version) => {
                let roll = json["sdk"]["rollForward"].as_str().unwrap_or("latestPatch");
                needs.push(Need {
                    policy: Some(format!("rollForward: {roll}")),
                    ..need(&base, version, &file)
                });
            }
            None => needs.extend(missing(base, &format!("{file} has no sdk.version"))),
        },
        Err(why) => needs.extend(missing(base, &why)),
    }
    needs
}

/// `actions/setup-node`: `node-version`, else `node-version-file`. Without both,
/// the action uses the node of the runner.
fn node(input: Input, workspace: &Path, base: Need) -> Vec<Need> {
    if let Some(version) = input("node-version") {
        return vec![need(&base, &version, "with.node-version")];
    }
    let Some(file) = input("node-version-file") else {
        return missing(base, "no node-version: the runner's node is used");
    };
    let text = match read_file(workspace, &file) {
        Ok(text) => text,
        Err(why) => return missing(base, &why),
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

/// A TOML string or array of strings, with `"` or `'` quotes and a trailing comment.
fn toml_strings(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '#' => break,
            '"' | '\'' => out.push(chars.by_ref().take_while(|&d| d != c).collect()),
            _ => {}
        }
    }
    out
}

/// The channel, components and targets of `rust-toolchain` or `rust-toolchain.toml`.
/// rustup reads the legacy file first when both are there.
fn toolchain_file(workspace: &Path) -> Option<(String, String, Vec<String>, Vec<String>)> {
    for name in ["rust-toolchain", "rust-toolchain.toml"] {
        let Ok(text) = read_file(workspace, name) else {
            continue;
        };
        if !text.contains('=') {
            return first_line(&text).map(|c| (c, name.to_string(), vec![], vec![]));
        }
        let (mut channel, mut components, mut targets) = (None, Vec::new(), Vec::new());
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let mut value = value.to_string();
            // An array can go over several lines.
            if value.trim_start().starts_with('[') {
                while !value.split('#').next().unwrap_or_default().contains(']') {
                    match lines.next() {
                        Some(next) => value.push_str(next.split('#').next().unwrap_or_default()),
                        None => break,
                    }
                }
            }
            match key.trim() {
                "channel" => channel = toml_strings(&value).into_iter().next(),
                "components" => components = toml_strings(&value),
                "targets" => targets = toml_strings(&value),
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
/// `actions-rust-lang/setup-rust-toolchain`, which reads the toolchain file first
/// and sets `RUSTFLAGS` (default `-D warnings`) and `CARGO_INCREMENTAL=0`.
fn rust(uses: &str, input: Input, workspace: &Path, base: Need) -> Vec<Need> {
    let components = split_list(input("components"));
    let targets = split_list(input("targets").or_else(|| input("target")));
    let lang = uses.to_ascii_lowercase().starts_with("actions-rust-lang/");
    let mut env = base.env.clone();
    if lang {
        let flags = input("rustflags").unwrap_or_else(|| "-D warnings".into());
        if !flags.is_empty() {
            env.insert("RUSTFLAGS".into(), flags);
        }
        env.insert("CARGO_INCREMENTAL".into(), "0".into());
    }
    let with = |request: String, source: String, extra: (Vec<String>, Vec<String>)| Need {
        components: [components.clone(), extra.0].concat(),
        targets: [targets.clone(), extra.1].concat(),
        exact: is_exact(&request) || is_dated(&request),
        request,
        source,
        env: env.clone(),
        ..base.clone()
    };
    if let Some(toolchain) = input("toolchain") {
        return vec![with(toolchain, "with.toolchain".into(), (vec![], vec![]))];
    }
    if !lang {
        let reference = uses.split_once('@').map(|(_, r)| r).unwrap_or_default();
        if reference.is_empty() || reference == "master" || reference.starts_with('v') {
            return missing(
                base,
                "dtolnay/rust-toolchain needs a toolchain input with this ref",
            );
        }
        return vec![with(
            reference.into(),
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

/// `nightly-2024-05-01`: one dated build.
fn is_dated(request: &str) -> bool {
    let parts: Vec<&str> = request.rsplit('-').take(3).collect();
    parts.len() == 3
        && request.matches('-').count() >= 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
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

/// `<tool>-version` (one version for each line), else `<tool>-version-file`.
fn simple(
    input: Input,
    workspace: &Path,
    base: Need,
    tool: &str,
    parse: fn(&str) -> Option<String>,
) -> Vec<Need> {
    let version_input = format!("{tool}-version");
    let file_input = format!("{tool}-version-file");
    if let Some(versions) = input(&version_input) {
        return versions
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|v| need(&base, v, &format!("with.{version_input}")))
            .collect();
    }
    let Some(file) = input(&file_input) else {
        return missing(
            base,
            &format!("no {version_input}: the runner's version is used"),
        );
    };
    let asdf = match tool {
        "go" => "golang ",
        other => &format!("{other} ")[..],
    };
    let name = Path::new(&file)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let read = read_file(workspace, &file).map(|t| match name.as_deref() {
        Some(".tool-versions") => t
            .lines()
            .find_map(|l| l.trim().strip_prefix(asdf).map(|v| v.trim().to_string())),
        Some("pyproject.toml") => t.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "requires-python")
                .then(|| toml_strings(v).into_iter().next())
                .flatten()
        }),
        _ => parse(&t),
    });
    match read {
        Ok(Some(v)) => vec![need(&base, &v, &file)],
        Ok(None) => missing(base, &format!("{file} names no version")),
        Err(why) => missing(base, &why),
    }
}

/// `actions/setup-java`: `distribution` and `java-version` (or `java-version-file`).
fn java(input: Input, workspace: &Path, base: Need) -> Vec<Need> {
    let distribution = input("distribution").unwrap_or_else(|| "(none)".into());
    let mut needs = simple(input, workspace, base, "java", |t| {
        t.lines().find_map(|l| {
            let l = l.trim();
            l.strip_prefix("java ")
                .map(|v| v.trim().to_string())
                .or_else(|| (!l.is_empty() && !l.starts_with('#')).then(|| l.to_string()))
        })
    });
    for n in &mut needs {
        n.policy = Some(format!("distribution: {distribution}"));
    }
    needs
}
