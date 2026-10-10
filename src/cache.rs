//! `--cache`: reuse a step that passed before on the same inputs (ADR-0010).
//!
//! Each run snapshots the working tree, untracked files included, as a Git tree object
//! through a temporary index. A passed step records that tree. A later run reuses the
//! pass when the two trees differ only in files that are not inputs of the step.

use crate::affected::is_input;
use crate::script::analyze;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Default)]
pub struct Entry {
    pub tree: String,
    pub at: u64,
    pub outputs: Map<String, Value>,
    pub env: Vec<(String, String)>,
    pub path: Vec<String>,
}

pub struct Cache {
    root: PathBuf,
    dir: PathBuf,
    tree: String,
    /// `<command> --version` output for each tool command, asked once per run.
    versions: Mutex<HashMap<String, String>>,
}

/// FNV-1a: a hash that stays the same across Rust releases, unlike `DefaultHasher`.
pub fn stable_hash(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

fn git(root: &Path, index: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let out = command.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Cache {
    /// Snapshots the working tree, or says why it cannot.
    pub fn open(root: &Path, run_dir: &Path) -> Result<Cache, String> {
        let common = git(
            root,
            None,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok_or("the checkout is not a Git repository")?;
        let real_index = git(
            root,
            None,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )
        .ok_or("Git does not report an index path")?;
        let index = run_dir.join("snapshot.index");
        // Starting from the real index lets Git reuse its file stat data.
        let _ = std::fs::copy(&real_index, &index);
        git(root, Some(&index), &["add", "-A", "--", "."])
            .ok_or("git add -A into a temporary index failed")?;
        let tree = git(root, Some(&index), &["write-tree"]).ok_or("git write-tree failed")?;
        let _ = std::fs::remove_file(&index);
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("LOCALAPPDATA"))
                    .map(|h| PathBuf::from(h).join(".cache"))
            })
            .ok_or("no cache folder: set XDG_CACHE_HOME or HOME")?;
        let dir = base.join("localci").join(stable_hash(&[&common]));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Cache {
            root: root.to_path_buf(),
            dir,
            tree,
            versions: Mutex::new(HashMap::new()),
        })
    }

    /// The versions of the tools that `script` runs, so a toolchain update is a miss.
    pub fn tool_versions(&self, script: &str) -> String {
        let Ok(usage) = analyze(script, &self.root, ".", 0) else {
            return String::new();
        };
        let mut known = self
            .versions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = String::new();
        for tool in usage.tools {
            let command = tool.commands[0];
            let version = known.entry(command.to_string()).or_insert_with(|| {
                Command::new(command)
                    .arg("--version")
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            });
            out.push_str(version);
            out.push('\n');
        }
        out
    }

    /// The identity of a step: everything that is not a file in the tree. `path` is the
    /// PATH the step sees; `versions` comes from `tool_versions`.
    #[allow(clippy::too_many_arguments)]
    pub fn key(
        script: &str,
        shell: Option<&str>,
        workdir: Option<&str>,
        env: &Map<String, Value>,
        host: &str,
        path: &str,
        versions: &str,
    ) -> String {
        let env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut parts = vec![
            script,
            shell.unwrap_or("-"),
            workdir.unwrap_or("-"),
            host,
            path,
            versions,
        ];
        parts.extend(env.iter().map(String::as_str));
        stable_hash(&parts)
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// A recorded pass that is still valid for the current tree.
    pub fn lookup(&self, key: &str, script: &str) -> Option<Entry> {
        let entry: Entry =
            serde_json::from_str(&std::fs::read_to_string(self.file(key)).ok()?).ok()?;
        if entry.tree == self.tree {
            return Some(entry);
        }
        let diff = git(
            &self.root,
            None,
            &[
                "diff",
                "--name-only",
                "--no-renames",
                &entry.tree,
                &self.tree,
            ],
        )?;
        let changed: Vec<&str> = diff.lines().filter(|l| !l.is_empty()).collect();
        let usage = analyze(script, &self.root, ".", 0).ok()?;
        if usage.tools.is_empty()
            || usage.writes_step_files
            || changed.iter().any(|f| usage.scripts.iter().any(|s| s == f))
        {
            return None;
        }
        let inert = changed.iter().all(|f| {
            usage
                .tools
                .iter()
                .all(|tool| !is_input(&self.root, tool, f))
        });
        inert.then_some(entry)
    }

    pub fn record(&self, key: &str, mut entry: Entry) {
        entry.tree = self.tree.clone();
        entry.at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Ok(text) = serde_json::to_string(&entry) {
            // Write and rename, so a parallel leg never reads half a record.
            let temp = self.dir.join(format!("{key}.{}.tmp", std::process::id()));
            if std::fs::write(&temp, text).is_ok()
                && std::fs::rename(&temp, self.file(key)).is_err()
            {
                let _ = std::fs::remove_file(&temp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_hash_is_fixed() {
        assert_eq!(
            super::stable_hash(&["a", "b"]),
            super::stable_hash(&["a", "b"])
        );
        assert_ne!(super::stable_hash(&["ab"]), super::stable_hash(&["a", "b"]));
        assert_eq!(super::stable_hash(&[]), "cbf29ce484222325");
    }
}
