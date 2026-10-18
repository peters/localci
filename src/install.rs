//! Installs the toolchains of the plan from their official sources (ADR-0012).
//!
//! Each download is checked against the checksum that its source publishes, in
//! memory, and the same bytes are unpacked. A mismatch stops the install. Nothing
//! runs with sudo, and nothing outside the toolchains folder changes.

use crate::toolchain::{Need, Tool};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// `LOCALCI_TOOLCHAINS`, else `$XDG_CACHE_HOME/localci/toolchains`, else
/// `~/.cache/localci/toolchains`. Empty variables do not count.
pub fn default_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("LOCALCI_TOOLCHAINS") {
        return Some(PathBuf::from(dir));
    }
    var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|base| base.join("localci").join("toolchains"))
}

/// `name` is safe as one folder name: letters, digits, `.`, `_`, `+` and `-`, a
/// letter or digit first, and no `..`. A version or toolchain name becomes a path,
/// so a workflow cannot point outside the toolchains folder.
pub fn safe_name(name: &str) -> Result<&str, String> {
    let ok = !name.contains("..")
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    if ok {
        Ok(name)
    } else {
        Err(format!("`{name}` is not a valid version or toolchain name"))
    }
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

/// Refuses `bytes` when their checksum is not `expected` (hex, any case).
pub fn verify(bytes: &[u8], sum: Sum, expected: &str, what: &str) -> Result<(), String> {
    let actual = hex_digest(bytes, sum);
    if expected.trim().is_empty() || !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "checksum mismatch for {what}: expected {}, got {actual}; the download is not used",
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

/// The host as the download names use it.
pub struct Platform {
    /// `linux` or `darwin`. The .NET names use `osx` for `darwin`.
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

/// A unique name part for temporary files and folders of this process.
pub fn unique() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

/// The lock of the toolchains folder: an OS file lock on `.lock`. A second setup
/// waits. A setup that stops, also with Ctrl-C or a kill, releases it.
pub struct Lock {
    _file: fs::File,
}

impl Lock {
    pub fn take(dir: &Path) -> Result<Lock, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(".lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Lock { _file: file }),
                Err(fs::TryLockError::WouldBlock) => {
                    if started.elapsed() > Duration::from_secs(1800) {
                        return Err(format!("{} is held by another setup", path.display()));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(fs::TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

/// The versions that `setup` chose for requests that are not exact, kept in
/// `requests.json` so that `run` finds them without the network.
pub fn remembered(dir: &Path, tool: Tool, request: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join("requests.json")).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let version = json[format!("{} {request}", tool.as_str())].as_str()?;
    safe_name(version).ok().map(String::from)
}

fn remember(dir: &Path, tool: Tool, request: &str, version: &str) -> Result<(), String> {
    let path = dir.join("requests.json");
    let mut json: Map<String, Value> = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{}: {e}; fix or delete it", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(format!("{}: {e}; fix or delete it", path.display())),
    };
    json.insert(
        format!("{} {request}", tool.as_str()),
        Value::String(version.into()),
    );
    let tmp = dir.join(format!(".requests-{}.json", unique()));
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&json).unwrap_or_default(),
    )
    .and_then(|_| fs::rename(&tmp, &path))
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// Unpacks gzip tar `bytes` into `final_dir` through a new temporary folder and a
/// rename, so a stopped setup never leaves a half folder that a run would use.
fn unpack(bytes: &[u8], final_dir: &Path, strip: bool) -> Result<(), String> {
    let parent = final_dir.parent().ok_or("no parent folder")?;
    let tmp = parent.join(format!(".tmp-{}", unique()));
    fs::create_dir(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut tar = Command::new("tar");
    tar.args(["-xzf", "-", "-C"])
        .arg(&tmp)
        .stdin(Stdio::piped());
    if strip {
        tar.arg("--strip-components=1");
    }
    let result = tar
        .spawn()
        .map_err(|e| format!("tar: {e}"))
        .and_then(|mut child| {
            let fed = child.stdin.take().map(|mut stdin| stdin.write_all(bytes));
            let status = child.wait().map_err(|e| format!("tar: {e}"))?;
            if let Some(Err(e)) = fed {
                return Err(format!("tar: {e}"));
            }
            if status.success() {
                Ok(())
            } else {
                Err(format!("tar could not unpack {}", final_dir.display()))
            }
        })
        .and_then(|_| {
            fs::rename(&tmp, final_dir).map_err(|e| format!("{}: {e}", final_dir.display()))
        });
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// Removes the temporary folders of a setup that a kill stopped. The caller holds
/// the lock, so no other setup uses them.
pub fn clean_leftovers(dir: &Path) {
    for folder in [dir.to_path_buf(), dir.join("node"), dir.join("rust")] {
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if [".tmp-", ".unpacked-", ".rustup-init-", ".requests-"]
                .iter()
                .any(|p| name.starts_with(p))
            {
                let _ = fs::remove_dir_all(entry.path()).or_else(|_| fs::remove_file(entry.path()));
            }
        }
    }
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
    // An exact version that is there needs no network. For other requests, the
    // remembered version is used when the source cannot be reached.
    if tool != Tool::Rust {
        if let Ok(a) = locate(need, dir, workspace) {
            if need.exact {
                return Ok(Installed {
                    version: a.version,
                    path: a.paths[0].clone(),
                    fresh: false,
                });
            }
        }
    }
    let platform = platform()?;
    let offline = |e: String| -> Result<Installed, String> {
        locate(need, dir, workspace)
            .map(|a| Installed {
                version: a.version,
                path: a.paths[0].clone(),
                fresh: false,
            })
            .map_err(|_| e)
    };
    match tool {
        Tool::Dotnet => {
            let (version, url, hash) =
                match crate::install_sources::dotnet(&need.request, &platform) {
                    Ok(found) => found,
                    Err(e) => return offline(e),
                };
            safe_name(&version)?;
            let fresh = dotnet_side_by_side(dir, &version, &url, &hash)?;
            remember(dir, tool, &need.request, &version)?;
            Ok(Installed {
                version,
                path: dir.join("dotnet"),
                fresh,
            })
        }
        Tool::Node => {
            let (version, url, hash) = match crate::install_sources::node(&need.request, &platform)
            {
                Ok(found) => found,
                Err(e) => return offline(e),
            };
            let target = dir.join("node").join(safe_name(&version)?);
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
    let bytes = fetch(url)?;
    verify(&bytes, sum, hash, url)?;
    unpack(&bytes, target, strip)?;
    Ok(true)
}

/// Installs one .NET SDK into the shared root `<dir>/dotnet`, as the official
/// install script and setup-dotnet do: versioned folders (`sdk/<v>`,
/// `shared/<fx>/<v>`, `host/fxr/<v>`) go next to the others, and only a newer SDK
/// replaces the top-level `dotnet` host. `.versions/<v>` marks a complete install.
fn dotnet_side_by_side(dir: &Path, version: &str, url: &str, hash: &str) -> Result<bool, String> {
    let root = dir.join("dotnet");
    let marker = root.join(".versions").join(version);
    if marker.is_file() {
        return Ok(false);
    }
    let unpacked = dir.join(format!(".unpacked-{}", unique()));
    archive(&unpacked, url, Sum::Sha512, hash, false)?;
    let newest = fs::read_dir(root.join(".versions"))
        .map(|rd| {
            rd.flatten()
                .all(|e| version_key(version) >= version_key(&e.file_name().to_string_lossy()))
        })
        .unwrap_or(true);
    let merged = fs::create_dir_all(root.join(".versions"))
        .map_err(|e| format!("{}: {e}", root.display()))
        .and_then(|_| merge(&unpacked, &root, newest));
    let _ = fs::remove_dir_all(&unpacked);
    merged?;
    fs::write(&marker, version).map_err(|e| format!("{}: {e}", marker.display()))?;
    Ok(true)
}

/// Numeric parts, then 1 for a release and 0 for a prerelease, then the prerelease
/// parts, so `10.0.100` > `10.0.100-rc.1` > `10.0.100-preview.7`.
type VersionKey = (Vec<u64>, u8, Vec<(u64, String)>);

fn version_key(v: &str) -> VersionKey {
    let (base, pre) = match v.split_once('-') {
        Some((base, pre)) => (base, Some(pre)),
        None => (v, None),
    };
    let numbers = base.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let pre_parts = pre
        .unwrap_or_default()
        .split('.')
        .map(|p| (p.parse().unwrap_or(0), p.to_string()))
        .collect();
    (numbers, u8::from(pre.is_none()), pre_parts)
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
    /// `rust 1.83.0`: a run adds it to `LOCALCI_TOOLCHAINS`, which is part of the
    /// cache key, so a channel update does not replay an old result.
    pub label: String,
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
    let mut activation = match tool {
        Tool::Dotnet | Tool::Node => {
            let version = if need.exact {
                safe_name(need.request.trim_start_matches('v'))?.to_string()
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
                let there = bin.join("node").is_file();
                (bin, vec![], there)
            };
            if !there {
                return Err(not_there());
            }
            Activation {
                version,
                paths: vec![bin],
                env,
                label: String::new(),
            }
        }
        Tool::Rust => {
            crate::install_sources::rust_activation(need, dir, workspace).ok_or_else(not_there)?
        }
        other => return Err(format!("{} is in the plan only", other.title())),
    };
    activation
        .env
        .extend(need.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    activation.label = format!("{} {}", tool.as_str(), activation.version);
    Ok(activation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_refuses_a_wrong_checksum() {
        let good = hex_digest(b"toolchain", Sum::Sha256);
        assert!(verify(b"toolchain", Sum::Sha256, &good, "x").is_ok());
        assert!(verify(b"toolchain", Sum::Sha256, &good.to_uppercase(), "x").is_ok());
        let bad = verify(
            b"toolchain",
            Sum::Sha256,
            &hex_digest(b"other", Sum::Sha256),
            "x",
        );
        assert!(bad.unwrap_err().contains("checksum mismatch"));
        assert!(verify(b"toolchain", Sum::Sha512, "", "x").is_err());
    }

    #[test]
    fn names_cannot_leave_the_folder() {
        for bad in [
            "../../x",
            "/abs/evil",
            "-x",
            "a/b",
            "20.0.0-/../x",
            "",
            ".hidden",
        ] {
            assert!(safe_name(bad).is_err(), "{bad}");
        }
        for good in [
            "8.0.404",
            "stable",
            "1.82.0",
            "nightly-2024-05-01",
            "20.0.0-rc.1",
        ] {
            assert!(safe_name(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn dotnet_sdks_merge_side_by_side() {
        let base = std::env::temp_dir().join(format!("localci-merge-{}", unique()));
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
        assert!(version_key("10.0.100") > version_key("10.0.100-rc.1"));
        assert!(version_key("10.0.100-rc.1") > version_key("10.0.100-preview.7"));
        assert!(version_key("10.0.100-rc.2") > version_key("10.0.100-rc.1"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_lock_is_released_when_its_holder_ends() {
        let dir = std::env::temp_dir().join(format!("localci-lock-{}", unique()));
        let first = Lock::take(&dir).unwrap();
        let file = fs::OpenOptions::new()
            .write(true)
            .open(dir.join(".lock"))
            .unwrap();
        assert!(matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)));
        drop(first);
        assert!(file.try_lock().is_ok());
        let _ = fs::remove_dir_all(&dir);
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
