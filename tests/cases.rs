//! Runs every folder in `tests/cases`. A folder holds `workflow.yml` and `case.json`:
//!
//! - `workflow.yml` may say `runs-on: localci-host`: the runner label of this host.
//! - `args`: the localci arguments. The run happens in a fresh Git repository.
//! - `exit`: the expected exit code (default 0).
//! - `expect`: JSON that stdout must contain. Objects match on the keys they name;
//!   arrays match item by item and must have the same length.
//! - `setup.sh` (optional file): runs in the repository first, for example to commit a
//!   base and then change files. Git has a test identity.
//! - `repeat`: how many times localci runs (default 1); `expect` checks the last run.
//! - `between.sh` (optional file): runs before each repeat after the first.
//! - An expected string that starts with `~` only has to be contained in the actual one.
//! - `hosts`: the host systems that run the case (default: all).
//! - `requires`: tools that must work first, for example `docker`.

use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn host() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn available(tool: &str) -> bool {
    let probe = match tool {
        "docker" => Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output(),
        other => Command::new(other).arg("--version").output(),
    };
    probe.is_ok_and(|o| o.status.success())
}

fn contains(expected: &Value, actual: &Value, at: &str, errors: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(want), Value::Object(have)) => {
            for (k, v) in want {
                contains(
                    v,
                    have.get(k).unwrap_or(&Value::Null),
                    &format!("{at}.{k}"),
                    errors,
                );
            }
        }
        (Value::Array(want), Value::Array(have)) if want.len() == have.len() => {
            for (i, (w, h)) in want.iter().zip(have).enumerate() {
                contains(w, h, &format!("{at}[{i}]"), errors);
            }
        }
        (Value::String(want), Value::String(have))
            if want.starts_with('~') && have.contains(&want[1..]) => {}
        _ if expected == actual => {}
        _ => errors.push(format!("{at}: expected {expected}, got {actual}")),
    }
}

fn run_case(dir: &Path) -> Result<Option<String>, String> {
    let case: Value = serde_json::from_str(
        &fs::read_to_string(dir.join("case.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("case.json: {e}"))?;
    let listed = |key: &str| -> Vec<String> {
        case[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let hosts = listed("hosts");
    if !hosts.is_empty() && !hosts.iter().any(|h| h == host()) {
        return Ok(Some(format!("not for {}", host())));
    }
    if let Some(tool) = listed("requires").into_iter().find(|t| !available(t)) {
        return Ok(Some(format!("{tool} is not available")));
    }
    let name = dir.file_name().unwrap_or_default();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cases")
        .join(name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join(".github/workflows")).map_err(|e| e.to_string())?;
    // `runs-on: localci-host` becomes the runner label of this host.
    let label = match host() {
        "windows" => "windows-latest",
        "macos" => "macos-latest",
        _ => "ubuntu-latest",
    };
    let workflow = fs::read_to_string(dir.join("workflow.yml")).map_err(|e| e.to_string())?;
    fs::write(
        work.join(".github/workflows/ci.yml"),
        workflow.replace("localci-host", label),
    )
    .map_err(|e| e.to_string())?;
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(&work)
        .status()
        .map_err(|e| e.to_string())?;
    let shell = |script: &str| -> Result<(), String> {
        let file = dir.join(script);
        if file.is_file() {
            let status = Command::new("sh")
                .args(["-eu".as_ref(), file.as_os_str()])
                .current_dir(&work)
                .envs([
                    ("GIT_AUTHOR_NAME", "localci"),
                    ("GIT_AUTHOR_EMAIL", "localci@example.invalid"),
                    ("GIT_COMMITTER_NAME", "localci"),
                    ("GIT_COMMITTER_EMAIL", "localci@example.invalid"),
                ])
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("{script} failed"));
            }
        }
        Ok(())
    };
    shell("setup.sh")?;
    // The step cache lives outside the repository, so it never shows up as a change.
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cases-cache")
        .join(name);
    let _ = fs::remove_dir_all(&cache);
    let args = listed("args");
    let mut out = None;
    for round in 0..case["repeat"].as_u64().unwrap_or(1) {
        if round > 0 {
            shell("between.sh")?;
        }
        out = Some(
            Command::new(env!("CARGO_BIN_EXE_localci"))
                .args(&args)
                .current_dir(&work)
                .env("XDG_CACHE_HOME", &cache)
                .output()
                .map_err(|e| e.to_string())?,
        );
    }
    let out = out.ok_or("repeat must be at least 1")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let want_exit = case["exit"].as_i64().unwrap_or(0);
    if out.status.code().map(i64::from) != Some(want_exit) {
        return Err(format!(
            "exit {:?}, expected {want_exit}\n{stderr}",
            out.status.code()
        ));
    }
    let actual: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("stdout is not JSON: {e}\n{stderr}"))?;
    let mut errors = Vec::new();
    contains(&case["expect"], &actual, "", &mut errors);
    if errors.is_empty() {
        Ok(None)
    } else {
        Err(format!("{}\n{actual:#}", errors.join("\n")))
    }
}

#[test]
fn every_case_folder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty());
    let mut failures = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        match run_case(dir) {
            Ok(None) => eprintln!("ok   {name}"),
            Ok(Some(why)) => eprintln!("skip {name}: {why}"),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
