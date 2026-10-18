//! The official sources of each toolchain and how a request becomes one version
//! (ADR-0012). The pick functions take parsed metadata, so tests need no network.

use crate::install::{
    fetch, fetch_json, safe_name, unique, verify, Activation, Installed, Platform, Sum,
};
use crate::toolchain::Need;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const DOTNET_INDEX: &str =
    "https://builds.dotnet.microsoft.com/dotnet/release-metadata/releases-index.json";
const NODE_DIST: &str = "https://nodejs.org/dist";
const RUSTUP_DIST: &str = "https://static.rust-lang.org/rustup/dist";

fn numbers(v: &str) -> Vec<u64> {
    v.split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// The channel (`8.0`) and, when the request does not name one SDK, how to pick it.
/// Exact `8.0.403`; feature band `8.0.4xx`; latest of a channel `8.0`, `8.0.x`; latest
/// of a major `8`, `8.x`.
pub fn dotnet_channel(request: &str, index: &Value) -> Result<(String, String), String> {
    let r = request.trim().trim_start_matches('v').to_ascii_lowercase();
    // A prerelease such as 8.0.100-rc.1 is one exact SDK of its channel.
    let parts: Vec<&str> = r.split('-').next().unwrap_or_default().split('.').collect();
    let channels = index["releases-index"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let channel_url = |ch: &str| {
        channels
            .iter()
            .find(|c| c["channel-version"].as_str() == Some(ch))
            .and_then(|c| c["releases.json"].as_str().map(String::from))
            .ok_or(format!(".NET channel {ch} is not in the release index"))
    };
    match parts.as_slice() {
        [major] | [major, "x"] => {
            let best = channels
                .iter()
                .filter_map(|c| c["channel-version"].as_str())
                .filter(|ch| ch.split('.').next() == Some(*major))
                .max_by_key(|ch| numbers(ch))
                .ok_or(format!("no .NET channel for {request}"))?;
            Ok((channel_url(best)?, format!("latest:{best}")))
        }
        [major, minor] | [major, minor, "x"] => {
            let ch = format!("{major}.{minor}");
            Ok((channel_url(&ch)?, format!("latest:{ch}")))
        }
        [major, minor, patch] => {
            let ch = format!("{major}.{minor}");
            if let (Some(band), false) = (patch.strip_suffix("xx"), r.contains('-')) {
                return Ok((channel_url(&ch)?, format!("band:{major}.{minor}.{band}")));
            }
            Ok((channel_url(&ch)?, format!("exact:{r}")))
        }
        _ => Err(format!("cannot read the .NET version `{request}`")),
    }
}

/// The SDK version, URL and SHA-512 for `rule` (from `dotnet_channel`) in a
/// channel's `releases.json`.
pub fn dotnet_sdk(
    rule: &str,
    releases: &Value,
    rid: &str,
) -> Result<(String, String, String), String> {
    let mut sdks: Vec<&Value> = Vec::new();
    for release in releases["releases"].as_array().into_iter().flatten() {
        match release["sdks"].as_array() {
            Some(list) => sdks.extend(list),
            None => sdks.extend(release.get("sdk")),
        }
    }
    fn version(s: &Value) -> String {
        s["version"].as_str().unwrap_or_default().to_string()
    }
    let stable: Vec<&Value> = sdks
        .iter()
        .copied()
        .filter(|s| !version(s).contains('-'))
        .collect();
    let chosen = if let Some(v) = rule.strip_prefix("exact:") {
        sdks.iter().copied().find(|s| version(s) == v)
    } else if let Some(prefix) = rule.strip_prefix("band:") {
        stable
            .iter()
            .copied()
            .filter(|s| {
                let v = version(s);
                v.starts_with(prefix) && v.rsplit('.').next().is_some_and(|p| p.len() == 3)
            })
            .max_by_key(|s| numbers(&version(s)))
    } else {
        stable.iter().copied().max_by_key(|s| numbers(&version(s)))
    };
    let sdk = chosen.ok_or(format!("no .NET SDK for {rule} in the channel"))?;
    let file = sdk["files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| {
            f["rid"].as_str() == Some(rid)
                && f["name"].as_str().is_some_and(|n| n.ends_with(".tar.gz"))
        })
        .ok_or(format!("no .NET SDK {} archive for {rid}", version(sdk)))?;
    let text = |k: &str| file[k].as_str().unwrap_or_default().to_string();
    Ok((version(sdk), text("url"), text("hash")))
}

pub fn dotnet(request: &str, p: &Platform) -> Result<(String, String, String), String> {
    let index = fetch_json(DOTNET_INDEX)?;
    let (url, rule) = dotnet_channel(request, &index)?;
    let releases = fetch_json(&url)?;
    let os = if p.os == "darwin" { "osx" } else { "linux" };
    dotnet_sdk(&rule, &releases, &format!("{os}-{}", p.arch))
}

/// The Node version for a request, from `https://nodejs.org/dist/index.json`. A range
/// such as `>=18` or `^20` is not resolved: only the action knows its rules.
pub fn node_pick(request: &str, index: &Value) -> Result<String, String> {
    let r = request.trim().to_ascii_lowercase();
    let r = r.trim_start_matches('v');
    let releases = index.as_array().cloned().unwrap_or_default();
    fn version(e: &Value) -> String {
        e["version"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('v')
            .to_string()
    }
    let newest = |keep: &dyn Fn(&Value) -> bool| {
        releases
            .iter()
            .filter(|e| keep(e))
            .max_by_key(|e| numbers(&version(e)))
            .map(version)
    };
    let found = if matches!(r, "latest" | "current" | "node") {
        newest(&|_| true)
    } else if let Some(name) = r.strip_prefix("lts/") {
        newest(&|e| match &e["lts"] {
            Value::String(codename) => name == "*" || codename.eq_ignore_ascii_case(name),
            _ => false,
        })
    } else if r.chars().any(|c| "<>=^~|* ".contains(c)) {
        return Err(format!(
            "Node range `{request}` is not resolved; pin a version"
        ));
    } else {
        let prefix = r.trim_end_matches(".x");
        newest(&|e| {
            let v = version(e);
            v == prefix || v.starts_with(&format!("{prefix}."))
        })
    };
    found.ok_or(format!("no Node.js release matches `{request}`"))
}

pub fn node(request: &str, p: &Platform) -> Result<(String, String, String), String> {
    let index = fetch_json(&format!("{NODE_DIST}/index.json"))?;
    let version = node_pick(request, &index)?;
    let name = format!("node-v{version}-{}-{}.tar.gz", p.os, p.arch);
    let sums = String::from_utf8_lossy(&fetch(&format!("{NODE_DIST}/v{version}/SHASUMS256.txt"))?)
        .into_owned();
    let hash = sums
        .lines()
        .find_map(|l| {
            l.split_once("  ")
                .filter(|(_, f)| f.trim() == name)
                .map(|(h, _)| h.to_string())
        })
        .ok_or(format!("{name} is not in SHASUMS256.txt"))?;
    let url = format!("{NODE_DIST}/v{version}/{name}");
    Ok((version, url, hash))
}

fn triple(p: &Platform) -> String {
    let arch = if p.arch == "x64" { "x86_64" } else { "aarch64" };
    if p.os == "darwin" {
        format!("{arch}-apple-darwin")
    } else {
        format!("{arch}-unknown-linux-gnu")
    }
}

/// Installs rustup into `<dir>/rust` (once, from a checked `rustup-init`) and the
/// toolchain of the request. rustup checks the toolchain downloads itself.
pub fn rust(need: &Need, dir: &Path, workspace: &Path) -> Result<Installed, String> {
    let p = crate::install::platform()?;
    let triple = triple(&p);
    let request = safe_name(&need.request)?;
    for name in need.components.iter().chain(&need.targets) {
        safe_name(name)?;
    }
    let home = dir.join("rust");
    let bin = home.join("cargo/bin");
    let env = [
        ("RUSTUP_HOME", home.join("rustup")),
        ("CARGO_HOME", home.join("cargo")),
    ];
    // rustup-init copies itself and then makes the proxies: all of them mark a
    // complete rustup, so a stopped init runs again.
    if !["rustup", "cargo", "rustc"]
        .iter()
        .all(|b| bin.join(b).is_file())
    {
        fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let url = format!("{RUSTUP_DIST}/{triple}/rustup-init");
        let bytes = fetch(&url)?;
        let sum = String::from_utf8_lossy(&fetch(&format!("{url}.sha256"))?).into_owned();
        verify(
            &bytes,
            Sum::Sha256,
            sum.split_whitespace().next().unwrap_or_default(),
            &url,
        )?;
        // rustup-init reads its role from its file name, so the name must stay.
        let init_dir = home.join(format!(".rustup-init-{}", unique()));
        fs::create_dir(&init_dir).map_err(|e| format!("{}: {e}", init_dir.display()))?;
        let init = init_dir.join("rustup-init");
        let ran = write_executable(&init, &bytes).and_then(|_| {
            let status = Command::new(&init)
                .args([
                    "-y",
                    "--no-modify-path",
                    "--profile",
                    "minimal",
                    "--default-toolchain",
                    "none",
                ])
                .envs(env.iter().map(|(k, v)| (k, v)))
                .status()
                .map_err(|e| format!("rustup-init: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err("rustup-init failed".to_string())
            }
        });
        let _ = fs::remove_dir_all(&init_dir);
        ran?;
    }
    let installed = rust_activation(need, dir, workspace).is_some();
    let mut cmd = Command::new(bin.join("rustup"));
    cmd.args(["toolchain", "install", request, "--profile", "minimal"]);
    for c in &need.components {
        cmd.args(["--component", c]);
    }
    for t in &need.targets {
        cmd.args(["--target", t]);
    }
    let status = cmd
        .envs(env.iter().map(|(k, v)| (k, v)))
        .status()
        .map_err(|e| format!("rustup: {e}"))?;
    if !status.success() {
        return Err(format!("rustup could not install {request}"));
    }
    let version = rust_activation(need, dir, workspace).map_or(request.to_string(), |a| a.version);
    Ok(Installed {
        version,
        path: toolchain_dir(dir, request, &triple),
        fresh: !installed,
    })
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o755);
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn toolchain_dir(dir: &Path, request: &str, triple: &str) -> std::path::PathBuf {
    let name = if request.ends_with(triple) {
        request.to_string()
    } else {
        format!("{request}-{triple}")
    };
    dir.join("rust/rustup/toolchains").join(name)
}

/// The version in the channel manifest that rustup writes at the end of an install,
/// for example `1.82.0`. Read from the file, so a plan runs nothing.
fn manifest_version(toolchain: &Path) -> Option<String> {
    let text =
        fs::read_to_string(toolchain.join("lib/rustlib/multirust-channel-manifest.toml")).ok()?;
    let mut in_rustc = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_rustc = line == "[pkg.rustc]";
        } else if let (true, Some(v)) = (in_rustc, line.strip_prefix("version = ")) {
            return v
                .trim_matches('"')
                .split_whitespace()
                .next()
                .map(String::from);
        }
    }
    None
}

/// The rustup proxies on `PATH`, `RUSTUP_HOME`, and `RUSTUP_TOOLCHAIN` unless the
/// repository has a toolchain file, which wins on GitHub too. `CARGO_HOME` stays the
/// user's, so the registry cache stays writable when the toolchains folder is not.
/// `None` until the toolchain and each requested component are complete.
pub fn rust_activation(need: &Need, dir: &Path, workspace: &Path) -> Option<Activation> {
    let p = crate::install::platform().ok()?;
    let triple = triple(&p);
    let tc = toolchain_dir(dir, safe_name(&need.request).ok()?, &triple);
    let version = manifest_version(&tc)?;
    let components = fs::read_to_string(tc.join("lib/rustlib/components")).unwrap_or_default();
    let has = |c: &String| {
        components.lines().any(|l| {
            l == format!("{c}-{triple}") || l.starts_with(&format!("{c}-preview-")) || l == c
        })
    };
    if !tc.join("bin/rustc").is_file() || !need.components.iter().all(has) {
        return None;
    }
    let mut env = vec![(
        "RUSTUP_HOME".to_string(),
        dir.join("rust/rustup").display().to_string(),
    )];
    let has_file = ["rust-toolchain", "rust-toolchain.toml"]
        .iter()
        .any(|f| workspace.join(f).is_file());
    if !has_file {
        env.push(("RUSTUP_TOOLCHAIN".into(), need.request.clone()));
    }
    Some(Activation {
        version,
        paths: vec![dir.join("rust/cargo/bin")],
        env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn index() -> Value {
        json!({"releases-index": [
            {"channel-version": "9.0", "releases.json": "u9"},
            {"channel-version": "8.0", "releases.json": "u8"},
            {"channel-version": "10.0", "releases.json": "u10"}]})
    }

    #[test]
    fn dotnet_requests() {
        assert_eq!(
            dotnet_channel("8.0.403", &index()).unwrap(),
            ("u8".into(), "exact:8.0.403".into())
        );
        assert_eq!(dotnet_channel("8.0.4xx", &index()).unwrap().1, "band:8.0.4");
        assert_eq!(dotnet_channel("8.0.x", &index()).unwrap().1, "latest:8.0");
        assert_eq!(
            dotnet_channel("10", &index()).unwrap(),
            ("u10".into(), "latest:10.0".into())
        );
        assert!(dotnet_channel("7.0", &index()).is_err());
        assert_eq!(
            dotnet_channel("8.0.100-rc.1", &index()).unwrap().1,
            "exact:8.0.100-rc.1"
        );
        let file = |v: &str| {
            json!({"version": v, "files": [
            {"name": "dotnet-sdk-linux-x64.tar.gz", "rid": "linux-x64", "url": format!("https://x/{v}"), "hash": "h"},
            {"name": "dotnet-sdk-linux-x64.zip", "rid": "linux-x64", "url": "zip", "hash": "z"}]})
        };
        let releases = json!({"releases": [
            {"sdks": [file("8.0.404"), file("8.0.311")]},
            {"sdks": [file("8.0.403"), file("8.0.100-rc.1")]}]});
        let pick = |rule: &str| dotnet_sdk(rule, &releases, "linux-x64").map(|r| r.0);
        assert_eq!(pick("exact:8.0.403").unwrap(), "8.0.403");
        assert_eq!(pick("band:8.0.3").unwrap(), "8.0.311");
        assert_eq!(pick("latest:8.0").unwrap(), "8.0.404");
        assert_eq!(
            dotnet_sdk("exact:8.0.404", &releases, "linux-x64")
                .unwrap()
                .1,
            "https://x/8.0.404"
        );
        assert!(dotnet_sdk("exact:8.0.403", &releases, "osx-arm64").is_err());
    }

    #[test]
    fn node_requests() {
        let index = json!([
            {"version": "v22.3.0", "lts": false},
            {"version": "v20.15.0", "lts": "Iron"},
            {"version": "v20.9.0", "lts": "Iron"},
            {"version": "v18.20.3", "lts": "Hydrogen"}]);
        assert_eq!(node_pick("20", &index).unwrap(), "20.15.0");
        assert_eq!(node_pick("v20.9.0", &index).unwrap(), "20.9.0");
        assert_eq!(node_pick("20.x", &index).unwrap(), "20.15.0");
        assert_eq!(node_pick("lts/*", &index).unwrap(), "20.15.0");
        assert_eq!(node_pick("lts/hydrogen", &index).unwrap(), "18.20.3");
        assert_eq!(node_pick("latest", &index).unwrap(), "22.3.0");
        assert!(node_pick(">=18", &index)
            .unwrap_err()
            .contains("not resolved"));
        assert!(node_pick("16", &index).is_err());
    }
}
