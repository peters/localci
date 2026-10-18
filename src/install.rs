//! Installs the toolchains of the plan from their official sources (ADR-0012).
//!
//! Each download is checked against the checksum that its source publishes. A
//! mismatch stops the install. Nothing runs with sudo, and nothing outside the
//! toolchains folder changes.

use crate::toolchain::{Need, Tool};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

/// `LOCALCI_TOOLCHAINS`, else `$XDG_CACHE_HOME/localci/toolchains`, else
/// `~/.cache/localci/toolchains`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LOCALCI_TOOLCHAINS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|base| base.join("localci").join("toolchains"))
}

/// The checksum algorithm that a source publishes.
#[derive(Clone, Copy)]
pub enum Sum {
    Sha256,
    Sha512,
}

pub fn hex_digest(bytes: &[u8], sum: Sum) -> String {
    let digest: Vec<u8> = match sum {
        Sum::Sha256 => Sha256::digest(bytes).to_vec(),
        Sum::Sha512 => Sha512::digest(bytes).to_vec(),
    };
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Refuses a file whose checksum is not `expected` (hex, any case).
pub fn verify(path: &Path, sum: Sum, expected: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let actual = hex_digest(&bytes, sum);
    if expected.trim().is_empty() || !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "checksum mismatch for {}: expected {}, got {actual}; the file is not used",
            path.display(),
            expected.trim()
        ));
    }
    Ok(())
}

/// The bytes at an https URL, through curl.
pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--retry",
            "3",
            url,
        ])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "download of {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

pub fn fetch_json(url: &str) -> Result<Value, String> {
    serde_json::from_slice(&fetch(url)?).map_err(|e| format!("{url}: {e}"))
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let bytes = fetch(url)?;
    fs::write(dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))
}

/// The host as the download names use it.
pub struct Platform {
    /// `linux`, `osx` (.NET) and `darwin` (Node) differ, so both are kept.
    pub os: &'static str,
    pub arch: &'static str,
}

pub fn platform() -> Result<Platform, String> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        return Err("localci setup installs toolchains on Linux and macOS only".into());
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return Err("localci setup installs toolchains on x64 and arm64 only".into());
    };
    Ok(Platform { os, arch })
}

/// One folder lock for the toolchains folder, so two setups do not install at once.
pub struct Lock(PathBuf);

impl Lock {
    pub fn take(dir: &Path) -> Result<Lock, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(".lock");
        let started = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {
                    let _ = fs::write(&path, std::process::id().to_string());
                    return Ok(Lock(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A lock older than one hour belongs to a setup that stopped.
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(3600));
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(1800) {
                        return Err(format!("{} is held by another setup", path.display()));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// The versions that `setup` chose for requests that are not exact, kept in
/// `requests.json` so that `run` finds them without the network.
pub fn remembered(dir: &Path, tool: Tool, request: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join("requests.json")).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    json[format!("{} {request}", tool.as_str())]
        .as_str()
        .map(String::from)
}

fn remember(dir: &Path, tool: Tool, request: &str, version: &str) -> Result<(), String> {
    let path = dir.join("requests.json");
    let mut json: Map<String, Value> = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    json.insert(
        format!("{} {request}", tool.as_str()),
        Value::String(version.into()),
    );
    let tmp = dir.join(format!(".requests-{}.json", std::process::id()));
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&json).unwrap_or_default(),
    )
    .and_then(|_| fs::rename(&tmp, &path))
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// Unpacks `archive` into `final_dir` through a temporary folder and a rename, so a
/// stopped setup never leaves a half folder that a run would use.
fn unpack(archive: &Path, final_dir: &Path, strip: bool) -> Result<(), String> {
    let parent = final_dir.parent().ok_or("no parent folder")?;
    let tmp = parent.join(format!(
        ".tmp-{}-{}",
        final_dir.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut tar = Command::new("tar");
    tar.arg("-xzf").arg(archive).arg("-C").arg(&tmp);
    if strip {
        tar.arg("--strip-components=1");
    }
    let status = tar.status().map_err(|e| format!("tar: {e}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&tmp);
        return Err(format!("tar could not unpack {}", archive.display()));
    }
    fs::rename(&tmp, final_dir).map_err(|e| format!("{}: {e}", final_dir.display()))
}

/// What `setup` did for one need.
pub struct Installed {
    pub version: String,
    pub path: PathBuf,
    pub fresh: bool,
}

/// Installs `need` into `dir`. The caller holds the lock.
pub fn install(need: &Need, dir: &Path, workspace: &Path) -> Result<Installed, String> {
    let tool = need.tool.ok_or("this setup action is not supported")?;
    if need.request.is_empty() {
        return Err(need.note.clone().unwrap_or_else(|| "no version".into()));
    }
    let platform = platform()?;
    match tool {
        Tool::Dotnet => {
            let (version, url, hash) = crate::install_sources::dotnet(&need.request, &platform)?;
            let fresh = dotnet_side_by_side(dir, &version, &url, &hash)?;
            remember(dir, tool, &need.request, &version)?;
            Ok(Installed {
                version,
                path: dir.join("dotnet"),
                fresh,
            })
        }
        Tool::Node => {
            let (version, url, hash) = crate::install_sources::node(&need.request, &platform)?;
            let target = dir.join("node").join(&version);
            let fresh = archive(&target, &url, Sum::Sha256, &hash, true)?;
            remember(dir, tool, &need.request, &version)?;
            Ok(Installed {
                version,
                path: target,
                fresh,
            })
        }
        Tool::Rust => crate::install_sources::rust(need, dir, workspace),
        other => Err(format!(
            "{} is in the plan only; localci setup does not install it yet",
            other.title()
        )),
    }
}

/// Downloads, checks and unpacks one archive, unless `target` is there. `true` when
/// it installed now.
fn archive(target: &Path, url: &str, sum: Sum, hash: &str, strip: bool) -> Result<bool, String> {
    if target.is_dir() {
        return Ok(false);
    }
    let parent = target.parent().ok_or("no parent folder")?;
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let file = parent.join(format!(".download-{}", std::process::id()));
    let result = download(url, &file)
        .and_then(|_| verify(&file, sum, hash))
        .and_then(|_| unpack(&file, target, strip));
    let _ = fs::remove_file(&file);
    result.map(|_| true)
}

/// Installs one .NET SDK into the shared root `<dir>/dotnet`, as the official
/// install script does: versioned folders (`sdk/<v>`, `shared/<fx>/<v>`,
/// `host/fxr/<v>`) go next to the others, and the top-level `dotnet` host is
/// replaced only by a newer SDK. `.versions/<v>` marks a complete install.
fn dotnet_side_by_side(dir: &Path, version: &str, url: &str, hash: &str) -> Result<bool, String> {
    let root = dir.join("dotnet");
    let marker = root.join(".versions").join(version);
    if marker.is_file() {
        return Ok(false);
    }
    let unpacked = dir.join(format!(".unpacked-dotnet-{version}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&unpacked);
    archive(&unpacked, url, Sum::Sha512, hash, false)?;
    let newest = fs::read_dir(root.join(".versions"))
        .map(|rd| {
            rd.flatten()
                .all(|e| version_key(version) >= version_key(&e.file_name().to_string_lossy()))
        })
        .unwrap_or(true);
    fs::create_dir_all(root.join(".versions")).map_err(|e| format!("{}: {e}", root.display()))?;
    let merged = merge(&unpacked, &root, newest);
    let _ = fs::remove_dir_all(&unpacked);
    merged?;
    fs::write(&marker, version).map_err(|e| format!("{}: {e}", marker.display()))?;
    Ok(true)
}

fn version_key(v: &str) -> Vec<u64> {
    v.split(['.', '-'])
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Moves what `dst` does not have from `src` into `dst`. A file that both have
/// stays, unless `replace` (the newest SDK) and it is at the top of the root.
fn merge(src: &Path, dst: &Path, replace: bool) -> Result<(), String> {
    for entry in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        let err = |e: std::io::Error| format!("{}: {e}", to.display());
        if fs::symlink_metadata(&to).is_err() {
            fs::rename(&from, &to).map_err(err)?;
        } else if from.is_dir() && to.is_dir() {
            merge(&from, &to, false)?;
        } else if replace && !from.is_dir() {
            fs::rename(&from, &to).map_err(err)?;
        }
    }
    Ok(())
}

/// The folders for `PATH` and the variables that make a run use an installed
/// toolchain, without the network.
#[derive(Debug, Default)]
pub struct Activation {
    pub version: String,
    pub paths: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
}

pub fn locate(need: &Need, dir: &Path, workspace: &Path) -> Result<Activation, String> {
    let tool = need.tool.ok_or("this setup action is not supported")?;
    if need.request.is_empty() {
        return Err(need.note.clone().unwrap_or_else(|| "no version".into()));
    }
    let not_there = || {
        format!(
            "{} {} is not in {} (run `localci setup`)",
            tool.title(),
            need.request,
            dir.display()
        )
    };
    match tool {
        Tool::Dotnet | Tool::Node => {
            let version = if need.exact {
                need.request.trim_start_matches('v').to_string()
            } else {
                remembered(dir, tool, &need.request).ok_or_else(not_there)?
            };
            let (bin, env, there) = if tool == Tool::Dotnet {
                // One root with every SDK side by side, as setup-dotnet makes it.
                let root = dir.join("dotnet");
                let there = root.join(".versions").join(&version).is_file();
                let env = vec![("DOTNET_ROOT".to_string(), root.display().to_string())];
                (root, env, there)
            } else {
                let bin = dir.join("node").join(&version).join("bin");
                let there = bin.is_dir();
                (bin, vec![], there)
            };
            if !there {
                return Err(not_there());
            }
            Ok(Activation {
                version,
                paths: vec![bin],
                env,
            })
        }
        Tool::Rust => {
            crate::install_sources::rust_activation(need, dir, workspace).ok_or_else(not_there)
        }
        other => Err(format!("{} is in the plan only", other.title())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_refuses_a_wrong_checksum() {
        let dir = std::env::temp_dir().join(format!("localci-verify-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("archive.tar.gz");
        fs::write(&file, b"toolchain").unwrap();
        let good = hex_digest(b"toolchain", Sum::Sha256);
        assert!(verify(&file, Sum::Sha256, &good).is_ok());
        assert!(verify(&file, Sum::Sha256, &good.to_uppercase()).is_ok());
        let bad = verify(&file, Sum::Sha256, &hex_digest(b"other", Sum::Sha256));
        assert!(bad.unwrap_err().contains("checksum mismatch"));
        assert!(verify(&file, Sum::Sha512, "").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dotnet_sdks_merge_side_by_side() {
        let base = std::env::temp_dir().join(format!("localci-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let make = |name: &str, sdk: &str, host: &str| {
            let d = base.join(name);
            fs::create_dir_all(d.join("sdk").join(sdk)).unwrap();
            fs::create_dir_all(d.join("shared/Microsoft.NETCore.App").join(sdk)).unwrap();
            fs::write(d.join("dotnet"), host).unwrap();
            d
        };
        let root = make("root", "8.0.404", "host 8");
        merge(&make("new", "10.0.100", "host 10"), &root, true).unwrap();
        merge(&make("old", "6.0.428", "host 6"), &root, false).unwrap();
        for sdk in ["8.0.404", "10.0.100", "6.0.428"] {
            assert!(root.join("sdk").join(sdk).is_dir(), "{sdk}");
            assert!(root.join("shared/Microsoft.NETCore.App").join(sdk).is_dir());
        }
        assert_eq!(fs::read_to_string(root.join("dotnet")).unwrap(), "host 10");
        assert!(version_key("10.0.100") > version_key("8.0.404"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn known_digests() {
        assert_eq!(
            hex_digest(b"abc", Sum::Sha256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(hex_digest(b"abc", Sum::Sha512).starts_with("ddaf35a193617aba"));
    }
}
