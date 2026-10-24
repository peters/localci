//! The version file of a checkout that has no setup step for a tool (ADR-0012).
//!
//! Most repositories declare the version of a tool in a file and use no setup
//! action. This module reads that file, with the parsers of the action path, so the
//! same file gives the same need with or without a step.

use crate::toolchain::{Need, Tool};
use crate::toolchain_inputs::{
    first_line, global_json, go_file, missing, need, read_file, toolchain_file,
};
use std::{collections::BTreeMap, path::Path};

/// The first of `names` that the checkout root has, with its text.
fn present(workspace: &Path, names: &[&str]) -> Option<(String, String)> {
    names.iter().find_map(|name| {
        read_file(workspace, name)
            .ok()
            .map(|text| ((*name).to_string(), text))
    })
}

/// A need from a version file: no action and no step, and `needs_for` sets the leg.
fn base_need(tool: Tool) -> Need {
    Need {
        leg: String::new(),
        step: 0,
        action: String::new(),
        tool: Some(tool),
        request: String::new(),
        source: String::new(),
        exact: false,
        policy: None,
        components: Vec::new(),
        targets: Vec::new(),
        note: None,
        env: BTreeMap::new(),
    }
}

/// One need from the first file of `names` that the checkout root has. A file that
/// names no version gives a note that names the file instead.
fn from_files(
    base: &Need,
    workspace: &Path,
    names: &[&str],
    parse: fn(&str) -> Option<String>,
) -> Vec<Need> {
    match present(workspace, names) {
        Some((name, text)) => match parse(&text) {
            Some(version) => vec![need(base, &version, &name)],
            None => missing(base.clone(), &format!("{name} names no version")),
        },
        None => Vec::new(),
    }
}

/// The toolchain file of the checkout root, by the parser of the action path, so the
/// same file gives the same need with or without a setup step.
fn rust_file(base: &Need, workspace: &Path) -> Vec<Need> {
    // rustup reads `rust-toolchain` first, as `toolchain_file` does.
    let Some((name, _)) = present(workspace, &["rust-toolchain", "rust-toolchain.toml"]) else {
        return Vec::new();
    };
    match toolchain_file(workspace) {
        Some((channel, file, components, targets)) => vec![Need {
            components,
            targets,
            ..need(base, &channel, &file)
        }],
        None => missing(base.clone(), &format!("{name} names no toolchain channel")),
    }
}

/// The need of a leg that has no setup step for `tool`, from one file of the checkout
/// root: most repositories declare the version in a file and use no action.
///
/// `needs_for` reads these files only for a tool that `localci setup` installs, so a
/// checkout with `.python-version` and no setup step gives no need for Python.
pub fn version_file(tool: Tool, workspace: &Path) -> Vec<Need> {
    let base = base_need(tool);
    match tool {
        Tool::Dotnet => match present(workspace, &["global.json"]) {
            Some((file, text)) => global_json(&file, &text, base),
            None => Vec::new(),
        },
        Tool::Node => from_files(&base, workspace, &[".nvmrc", ".node-version"], first_line),
        Tool::Rust => rust_file(&base, workspace),
        Tool::Go => from_files(&base, workspace, &["go.mod"], |text| {
            // A `go.mod` always has a `go` line. The first line is `module ...`, which
            // is not a version.
            go_file(text).filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
        }),
        Tool::Python => from_files(&base, workspace, &[".python-version"], first_line),
        // There is no version file for Java in this plan; an action input names it.
        Tool::Java => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    /// A checkout root with one file for each pair, and its text.
    fn checkout(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("localci-vf-{name}-{}", crate::install::unique()));
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    #[test]
    fn a_version_file_gives_one_need_for_each_tool() {
        let dir = checkout(
            "all",
            &[
                (
                    "global.json",
                    r#"{ "sdk": { "version": "8.0.404", "rollForward": "latestFeature" } }"#,
                ),
                (".nvmrc", "# pinned\n\n20.11.1\n"),
                (
                    "rust-toolchain.toml",
                    "[toolchain]\nchannel = \"1.81.0\"\ncomponents = [\"rustfmt\"]\n",
                ),
                ("go.mod", "module example\n\ngo 1.22\ntoolchain go1.23.4\n"),
                (".python-version", "3.12\n"),
            ],
        );
        for (tool, request, source, exact) in [
            (Tool::Dotnet, "8.0.404", "global.json", true),
            (Tool::Node, "20.11.1", ".nvmrc", true),
            (Tool::Rust, "1.81.0", "rust-toolchain.toml", true),
            (Tool::Go, "1.23.4", "go.mod", true),
            (Tool::Python, "3.12", ".python-version", false),
        ] {
            let needs = version_file(tool, &dir);
            assert_eq!(needs.len(), 1, "{tool:?}");
            let need = &needs[0];
            assert_eq!(need.request, request, "{tool:?}");
            assert_eq!(need.source, source, "{tool:?}");
            assert_eq!(need.exact, exact, "{tool:?}");
            assert_eq!(need.tool, Some(tool));
            assert_eq!(need.step, 0);
            assert!(need.action.is_empty() && need.leg.is_empty());
        }
        assert_eq!(
            version_file(Tool::Dotnet, &dir)[0].policy.as_deref(),
            Some("rollForward: latestFeature")
        );
        assert_eq!(
            version_file(Tool::Rust, &dir)[0].components,
            ["rustfmt".to_string()]
        );
        assert!(version_file(Tool::Java, &dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_file_of_each_tool_wins() {
        let dir = checkout(
            "first",
            &[
                (".nvmrc", "18.20.4\n"),
                (".node-version", "20.11.1\n"),
                ("rust-toolchain", "1.82.0\n"),
                ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.81.0\"\n"),
                ("go.mod", "module example\n\ngo 1.22\n"),
                (".python-version", "3.13\n"),
            ],
        );
        // rustup reads `rust-toolchain` before `rust-toolchain.toml`.
        for (tool, request, source) in [
            (Tool::Node, "18.20.4", ".nvmrc"),
            (Tool::Rust, "1.82.0", "rust-toolchain"),
            (Tool::Go, "1.22", "go.mod"),
            (Tool::Python, "3.13", ".python-version"),
        ] {
            assert_eq!(version_file(tool, &dir)[0].request, request, "{tool:?}");
            assert_eq!(version_file(tool, &dir)[0].source, source, "{tool:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_without_a_version_gives_a_note() {
        let dir = checkout(
            "notes",
            &[
                ("global.json", r#"{ "sdk": { "rollForward": "minor" } }"#),
                (".nvmrc", "# only a comment\n"),
                ("go.mod", "module example\n"),
                (
                    "rust-toolchain.toml",
                    "[toolchain]\nprofile = \"minimal\"\n",
                ),
            ],
        );
        for (tool, note) in [
            (Tool::Dotnet, "global.json has no sdk.version"),
            (Tool::Node, ".nvmrc names no version"),
            (Tool::Rust, "rust-toolchain.toml names no toolchain channel"),
            (Tool::Go, "go.mod names no version"),
        ] {
            let needs = version_file(tool, &dir);
            assert_eq!(needs.len(), 1, "{tool:?}");
            assert_eq!(needs[0].note.as_deref(), Some(note), "{tool:?}");
            assert!(needs[0].request.is_empty() && needs[0].source.is_empty());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_checkout_without_a_version_file_gives_no_need() {
        let dir = checkout("empty", &[]);
        for tool in [Tool::Dotnet, Tool::Node, Tool::Rust, Tool::Go, Tool::Python] {
            assert!(version_file(tool, &dir).is_empty(), "{tool:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_global_json_that_does_not_parse_gives_a_note() {
        let dir = checkout(
            "broken",
            &[("global.json", "{ \"sdk\": "), (".node-version", "20\n")],
        );
        let needs = version_file(Tool::Dotnet, &dir);
        assert_eq!(needs.len(), 1);
        assert!(needs[0]
            .note
            .as_deref()
            .is_some_and(|n| n.starts_with("global.json: ")));
        // `.node-version` is used when `.nvmrc` is absent.
        assert_eq!(version_file(Tool::Node, &dir)[0].source, ".node-version");
        let _ = fs::remove_dir_all(&dir);
    }
}
