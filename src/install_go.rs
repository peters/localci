//! Go: the official release metadata, the archive and its activation (ADR-0012).
//!
//! The pick function takes parsed metadata, so tests need no network. The bytes of
//! the archive are checked against the SHA-256 of the metadata in memory, and the
//! same bytes are unpacked.

use crate::install::{
    fetch_json, platform, remembered, safe_name, Activation, Installed, Platform, Sum,
};
use crate::install_sources::numbers;
use crate::toolchain::{Need, Tool};
use serde_json::Value;
use std::path::Path;

const GO_DL: &str = "https://go.dev/dl";
/// Every release, so a pinned older version resolves too.
const METADATA: &str = "https://go.dev/dl/?mode=json&include=all";

/// The version text without an optional `go` or `v` prefix: `go1.23.4` is `1.23.4`.
fn bare(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix("go")
        .unwrap_or(text)
        .trim_start_matches('v')
}

/// The architecture as the Go archive names it.
fn go_arch(arch: &str) -> Result<&'static str, String> {
    match arch {
        "x64" => Ok("amd64"),
        "arm64" => Ok("arm64"),
        other => Err(format!(
            "Go has no archive for the architecture `{other}`; localci installs x64 and arm64"
        )),
    }
}

/// The newest stable release that matches `request`, and the name and SHA-256 of its
/// archive for `os` and `arch` (`linux`, `darwin`, `amd64`, `arm64`). A request is an
/// exact version such as `1.23.4` or a minor line such as `1.23`.
pub fn pick(
    request: &str,
    index: &Value,
    os: &str,
    arch: &str,
) -> Result<(String, String, String), String> {
    // `1.22.x` is the line `1.22`, and `stable` and `oldstable` are the two newest lines, as the
    // setup action reads them.
    let want = bare(request).trim_end_matches(".x").to_string();
    let releases = index.as_array().cloned().unwrap_or_default();
    let version = |e: &Value| bare(e["version"].as_str().unwrap_or_default()).to_string();
    let stable = |e: &Value| e["stable"].as_bool().unwrap_or(false);
    let line = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
    let newest = |keep: &dyn Fn(&str) -> bool| {
        releases
            .iter()
            .filter(|e| stable(e) && keep(&version(e)))
            .max_by_key(|e| numbers(&version(e)))
    };
    let chosen = if matches!(want.as_str(), "stable" | "oldstable") {
        let mut lines: Vec<String> = releases
            .iter()
            .filter(|e| stable(e))
            .map(|e| line(&version(e)))
            .collect();
        lines.sort_by_key(|l| numbers(l));
        lines.dedup();
        let pick = if want == "stable" {
            lines.last()
        } else {
            lines.iter().rev().nth(1)
        };
        pick.and_then(|l| newest(&|v| &line(v) == l))
    } else {
        newest(&|v| v == want || v.starts_with(&format!("{want}.")))
    };
    let release = match chosen {
        Some(release) => release,
        None if releases.iter().any(|e| {
            let v = version(e);
            v == want || v.starts_with(&format!("{want}."))
        }) =>
        {
            return Err(format!(
                "no stable Go release matches `{request}`; only a prerelease does"
            ))
        }
        None => return Err(format!("no Go release matches `{request}`")),
    };
    let version = version(release);
    let name = format!("go{version}.{os}-{arch}.tar.gz");
    let file = release["files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f["filename"].as_str() == Some(name.as_str()))
        .ok_or(format!("no Go {version} archive for {os}-{arch}"))?;
    let hash = file["sha256"].as_str().unwrap_or_default().to_string();
    Ok((version, format!("{GO_DL}/{name}"), hash))
}

/// The version, URL and SHA-256 of `request`, from the official metadata.
pub fn release(request: &str, p: &Platform) -> Result<(String, String, String), String> {
    let index = fetch_json(METADATA)?;
    pick(request, &index, p.os, go_arch(p.arch)?)
}

/// Installs the Go of `need` into `<dir>/go/<version>`, which holds the `go` folder
/// of the archive, so `bin/go` is at its top. When the metadata cannot be reached,
/// the installed version of the request is used (decision 8). A wrong checksum or a
/// failed unpack stops the install.
pub fn install(need: &Need, dir: &Path) -> Result<Installed, String> {
    let (version, url, hash) = match release(&need.request, &platform()?) {
        Ok(found) => found,
        Err(reason) => {
            let a = activation(need, dir).ok_or(reason)?;
            return Ok(Installed {
                path: dir.join("go").join(&a.version),
                version: a.version,
                fresh: false,
            });
        }
    };
    let target = dir.join("go").join(safe_name(&version)?);
    let fresh = crate::install::archive(&target, &url, Sum::Sha256, &hash, true)?;
    Ok(Installed {
        version,
        path: target,
        fresh,
    })
}

/// `PATH` gets `<dir>/go/<version>/bin` and the step gets `GOROOT`, so a build finds
/// its standard library too. `None` until `bin/go` is there.
pub fn activation(need: &Need, dir: &Path) -> Option<Activation> {
    let version = if need.exact {
        safe_name(bare(&need.request)).ok()?.to_string()
    } else {
        remembered(dir, Tool::Go, &need.request)?
    };
    let root = dir.join("go").join(&version);
    if !root.join("bin/go").is_file() {
        return None;
    }
    Some(Activation {
        version,
        paths: vec![root.join("bin")],
        env: vec![("GOROOT".to_string(), root.display().to_string())],
        label: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    fn index() -> Value {
        let file = |name: &str, os: &str, arch: &str, hash: &str| {
            serde_json::json!({
                "filename": name, "os": os, "arch": arch, "sha256": hash, "kind": "archive"})
        };
        let release = |version: &str, stable: bool, hash: &str| {
            serde_json::json!({"version": version, "stable": stable, "files": [
                file(&format!("{version}.linux-amd64.tar.gz"), "linux", "amd64", hash),
                file(&format!("{version}.linux-arm64.tar.gz"), "linux", "arm64", "arm"),
                file(&format!("{version}.darwin-arm64.tar.gz"), "darwin", "arm64", "mac")]})
        };
        serde_json::json!([
            release("go1.24rc1", false, "rc"),
            release("go1.23.4", true, "newest"),
            release("go1.23.3", true, "old"),
            release("go1.22.10", true, "line22")
        ])
    }

    #[test]
    fn go_requests() {
        let resolve = |request: &str| pick(request, &index(), "linux", "amd64");
        assert_eq!(
            resolve("1.23.4").unwrap(),
            (
                "1.23.4".into(),
                "https://go.dev/dl/go1.23.4.linux-amd64.tar.gz".into(),
                "newest".into()
            )
        );
        // The `go` line of go.mod names a minor line, the toolchain line a version.
        assert_eq!(resolve("1.23").unwrap().0, "1.23.4");
        assert_eq!(resolve("1.22").unwrap().0, "1.22.10");
        assert_eq!(resolve("go1.23.3").unwrap().2, "old");
        assert!(resolve("1.23.4").unwrap().1.ends_with("linux-amd64.tar.gz"));
        // The setup action also takes a line with `.x`, and the two channel names.
        assert_eq!(resolve("1.22.x").unwrap().0, "1.22.10");
        assert_eq!(resolve("stable").unwrap().0, "1.23.4");
        assert_eq!(resolve("oldstable").unwrap().0, "1.22.10");
        assert!(resolve("1.24rc1")
            .unwrap_err()
            .contains("no stable Go release matches"));
        assert!(resolve("1.19")
            .unwrap_err()
            .contains("no Go release matches"));
        let arm = pick("1.23", &index(), "linux", "arm64").unwrap();
        assert_eq!(arm.1, "https://go.dev/dl/go1.23.4.linux-arm64.tar.gz");
        let mac = pick("1.23", &index(), "darwin", "arm64").unwrap();
        assert_eq!(mac.1, "https://go.dev/dl/go1.23.4.darwin-arm64.tar.gz");
        assert_eq!(go_arch("x64").unwrap(), "amd64");
        assert!(go_arch("x86").unwrap_err().contains("no archive"));
    }

    #[test]
    fn the_archive_lands_at_bin_go() {
        let base = std::env::temp_dir().join(format!("localci-go-{}", crate::install::unique()));
        let src = base.join("archive/go/bin");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("go"), "#!/bin/sh\n").unwrap();
        let tar = base.join("go.tar.gz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&tar)
            .args(["-C"])
            .arg(base.join("archive"))
            .arg("go")
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = fs::read(&tar).unwrap();
        let target = base.join("go/1.23.4");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        crate::install::unpack(&bytes, &target, true).unwrap();
        assert!(target.join("bin/go").is_file());
        assert!(base.join("go/1.23.4").is_dir());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn activation_sets_goroot() {
        let dir = std::env::temp_dir().join(format!("localci-go-act-{}", crate::install::unique()));
        let bin = dir.join("go/1.23.4/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("go"), "#!/bin/sh\n").unwrap();
        let need = |request: &str, exact: bool| Need {
            leg: "build".into(),
            step: 1,
            action: "actions/setup-go@v5".into(),
            tool: Some(Tool::Go),
            request: request.into(),
            source: "with.go-version".into(),
            exact,
            policy: None,
            components: vec![],
            targets: vec![],
            note: None,
            env: Default::default(),
        };
        let a = activation(&need("1.23.4", true), &dir).unwrap();
        assert_eq!(a.version, "1.23.4");
        assert_eq!(a.paths, vec![dir.join("go").join("1.23.4").join("bin")]);
        assert_eq!(
            a.env,
            vec![(
                "GOROOT".to_string(),
                dir.join("go").join("1.23.4").display().to_string()
            )]
        );
        assert!(activation(&need("1.23.3", true), &dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
