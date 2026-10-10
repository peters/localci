//! `--cache`: reuse a step that passed before on the same inputs (ADR-0010).
//!
//! Each run snapshots the working tree, untracked files included, as a Git tree object
//! through a temporary index. A passed step records that tree. A later run reuses the
//! pass when the two trees differ only in files that are not inputs of the step.

use crate::affected::{analyze, is_input};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
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
    /// Snapshots the working tree. `None` outside a Git repository.
    pub fn open(root: &Path, run_dir: &Path) -> Option<Cache> {
        let common = git(
            root,
            None,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let real_index = git(
            root,
            None,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )?;
        let index = run_dir.join("snapshot.index");
        // Starting from the real index lets Git reuse its file stat data.
        let _ = std::fs::copy(&real_index, &index);
        git(root, Some(&index), &["add", "-A", "--", "."])?;
        let tree = git(root, Some(&index), &["write-tree"])?;
        let _ = std::fs::remove_file(&index);
        let mut hasher = DefaultHasher::new();
        common.hash(&mut hasher);
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("LOCALAPPDATA"))
                    .map(|h| PathBuf::from(h).join(".cache"))
            })?;
        let dir = base
            .join("localci")
            .join(format!("{:016x}", hasher.finish()));
        std::fs::create_dir_all(&dir).ok()?;
        Some(Cache {
            root: root.to_path_buf(),
            dir,
            tree,
        })
    }

    /// The identity of a step: everything that is not a file in the tree.
    pub fn key(
        script: &str,
        shell: Option<&str>,
        workdir: Option<&str>,
        env: &Map<String, Value>,
        host: &str,
    ) -> String {
        let mut hasher = DefaultHasher::new();
        (script, shell, workdir, host).hash(&mut hasher);
        for (k, v) in env {
            (k, v.to_string()).hash(&mut hasher);
        }
        format!("{:016x}", hasher.finish())
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
        let usage = analyze(script, &self.root, 0).ok()?;
        if usage.tools.is_empty() || changed.iter().any(|f| usage.scripts.iter().any(|s| s == f)) {
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
            let _ = std::fs::write(self.file(key), text);
        }
    }
}
