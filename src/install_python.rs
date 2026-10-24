//! Python: the official standalone builds, the archive and its activation (ADR-0012).
//!
//! The pick function takes the parsed manifest, so tests need no network. The manifest
//! of `actions/python-versions` names each file of each release, and the release
//! publishes the SHA-256 of each file in its `hashes.sha256`.

use crate::install::{
    fetch, fetch_json, platform, remembered, safe_name, Activation, Installed, Platform, Sum,
};
use crate::install_sources::numbers;
use crate::toolchain::{Need, Tool};
use serde_json::Value;
use std::path::Path;

/// The manifest of `actions/python-versions`, which is the source of
/// `actions/setup-python`. It lists every release, so a pinned older version resolves too.
const MANIFEST: &str =
    "https://raw.githubusercontent.com/actions/python-versions/main/versions-manifest.json";

/// The reason that `request` is not a version, with the text of the request. A range or
/// a specifier needs the setup action, which knows its rules (decision 4).
fn refuse(request: &str) -> String {
    let request = request.trim();
    if request.chars().any(|c| "<>=^~|* ".contains(c)) {
        format!(
            "Python range or specifier `{request}` is not resolved; pin a version such as 3.12.7"
        )
    } else {
        format!("Python `{request}` is not a version; name a version such as 3.12.7")
    }
}

/// The numeric parts of a request. `x` at the end matches every patch, so `3.x` is the
/// newest 3 and `3.12.x` the newest of `3.12`. A range or a specifier is not a version.
fn wanted(request: &str) -> Result<Vec<String>, String> {
    let parts: Vec<String> = request
        .trim()
        .split('.')
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let mut keep = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if part == "x" && last && i > 0 {
            break;
        }
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(refuse(request));
        }
        keep.push(part.clone());
    }
    if keep.is_empty() || keep.len() > 3 {
        return Err(refuse(request));
    }
    Ok(keep)
}

/// The newest stable Python that matches `request`, and the download URL of its archive
/// for `os` and `arch` on the newest Ubuntu that the manifest holds for that version, so
/// the interpreter links the OpenSSL of a current host. A request is an exact version
/// (`3.12.7`), a minor line (`3.12`), or a line with `x` (`3.12.x`, `3.x`).
pub fn pick(
    request: &str,
    manifest: &Value,
    os: &str,
    arch: &str,
) -> Result<(String, String), String> {
    let want = wanted(request)?;
    let releases = manifest.as_array().cloned().unwrap_or_default();
    let version = |e: &Value| e["version"].as_str().unwrap_or_default().to_string();
    let release = releases
        .iter()
        .filter(|e| e["stable"].as_bool().unwrap_or(false))
        .filter(|e| {
            let v = version(e);
            let parts: Vec<&str> = v.split('.').collect();
            parts.len() == 3 && want.iter().zip(&parts).all(|(w, p)| w.as_str() == *p)
        })
        .max_by_key(|e| numbers(&version(e)))
        .ok_or(format!("no stable Python release matches `{request}`"))?;
    let version = version(release);
    // `arch` excludes the `-freethreaded` files; a darwin file has no `platform_version`.
    let chosen = release["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["platform"].as_str() == Some(os) && f["arch"].as_str() == Some(arch))
        .filter_map(|f| {
            let url = f["download_url"].as_str()?;
            let ubuntu = numbers(f["platform_version"].as_str().unwrap_or_default());
            Some((ubuntu, url.to_string()))
        })
        .max_by_key(|(ubuntu, _)| ubuntu.clone())
        .ok_or(format!("no Python {version} archive for {os} {arch}"))?;
    Ok((version, chosen.1))
}

/// The SHA-256 of `name` in the text of a `hashes.sha256`, which has one
/// `<hash> <file>` line for each file of the release.
fn sha256_of(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim() == name).then(|| hash.trim().to_string())
    })
}

/// The SHA-256 of the archive at `url`, from the `hashes.sha256` of its release.
fn checksum(url: &str) -> Result<String, String> {
    let (folder, name) = url
        .rsplit_once('/')
        .ok_or_else(|| format!("{url} has no file name"))?;
    let text = String::from_utf8_lossy(&fetch(&format!("{folder}/hashes.sha256"))?).into_owned();
    sha256_of(&text, name).ok_or_else(|| format!("{name} is not in {folder}/hashes.sha256"))
}

/// The version, URL and SHA-256 of `request`, from the official manifest.
pub fn release(request: &str, p: &Platform) -> Result<(String, String, String), String> {
    let manifest = fetch_json(MANIFEST)?;
    let (version, url) = pick(request, &manifest, p.os, p.arch)?;
    let hash = checksum(&url)?;
    Ok((version, url, hash))
}

/// Installs the Python of `need` into `<dir>/python/<version>`, which holds `bin/python3`
/// at its top. When the manifest cannot be reached, the installed version of the request
/// is used (decision 8). A wrong checksum or a failed unpack stops the install.
pub fn install(need: &Need, dir: &Path) -> Result<Installed, String> {
    let (version, url, hash) = match release(&need.request, &platform()?) {
        Ok(found) => found,
        Err(reason) => {
            let a = activation(need, dir).ok_or(reason)?;
            return Ok(Installed {
                path: dir.join("python").join(&a.version),
                version: a.version,
                fresh: false,
            });
        }
    };
    let target = dir.join("python").join(safe_name(&version)?);
    let fresh = crate::install::archive(&target, &url, Sum::Sha256, &hash, true)?;
    Ok(Installed {
        version,
        path: target,
        fresh,
    })
}

/// The `bin` of the version on `PATH`. On Linux the build also needs the `lib` of the
/// version in `LD_LIBRARY_PATH`, because `bin/python3` links `libpython3.x.so`, as the
/// setup action sets it. `None` until `bin/python3` is there.
pub fn activation(need: &Need, dir: &Path) -> Option<Activation> {
    let version = if need.exact {
        safe_name(&need.request).ok()?.to_string()
    } else {
        remembered(dir, Tool::Python, &need.request)?
    };
    let root = dir.join("python").join(&version);
    if !root.join("bin/python3").is_file() {
        return None;
    }
    let mut env = Vec::new();
    if cfg!(target_os = "linux") {
        env.push((
            "LD_LIBRARY_PATH".to_string(),
            root.join("lib").display().to_string(),
        ));
    }
    Some(Activation {
        version,
        paths: vec![root.join("bin")],
        env,
        label: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    /// One file of the manifest. A linux file has its Ubuntu in the name, a darwin file
    /// has no `platform_version`.
    fn file(version: &str, arch: &str, platform: &str, ubuntu: Option<&str>) -> Value {
        let name = match ubuntu {
            Some(ubuntu) => format!("python-{version}-{platform}-{ubuntu}-{arch}.tar.gz"),
            None => format!("python-{version}-{platform}-{arch}.tar.gz"),
        };
        let mut file = json!({
            "filename": name,
            "arch": arch,
            "platform": platform,
            "download_url": format!("https://example.invalid/releases/{name}"),
        });
        if let Some(ubuntu) = ubuntu {
            file["platform_version"] = json!(ubuntu);
        }
        file
    }

    fn release(version: &str, stable: bool, files: Vec<Value>) -> Value {
        json!({"version": version, "stable": stable, "files": files})
    }

    fn manifest() -> Value {
        json!([
            release(
                "3.13.0-rc.1",
                false,
                vec![file("3.13.0-rc.1", "x64", "linux", Some("22.04"))]
            ),
            release(
                "3.13.0",
                true,
                vec![file("3.13.0", "x64", "linux", Some("22.04"))]
            ),
            release(
                "3.12.7",
                true,
                vec![
                    file("3.12.7", "x64", "linux", Some("20.04")),
                    file("3.12.7", "x64", "linux", Some("22.04")),
                    file("3.12.7", "x64-freethreaded", "linux", Some("22.04")),
                    file("3.12.7", "arm64", "linux", Some("22.04")),
                    file("3.12.7", "arm64", "darwin", None),
                    file("3.12.7", "x64", "darwin", None),
                ]
            ),
            release(
                "3.12.3",
                true,
                vec![file("3.12.3", "x64", "linux", Some("22.04"))]
            ),
            release(
                "3.10.1",
                true,
                vec![file("3.10.1", "arm64", "darwin", None)]
            ),
        ])
    }

    #[test]
    fn python_requests() {
        let resolve = |request: &str| pick(request, &manifest(), "linux", "x64");
        let (version, url) = resolve("3.12.7").unwrap();
        assert_eq!(version, "3.12.7");
        // The newest Ubuntu of that version, so the interpreter links a current OpenSSL,
        // and never the freethreaded file.
        assert!(
            url.ends_with("/python-3.12.7-linux-22.04-x64.tar.gz"),
            "{url}"
        );
        // A minor line, and its `.x` form, take the newest patch of the line.
        assert_eq!(resolve("3.12").unwrap().0, "3.12.7");
        assert_eq!(resolve("3.12.x").unwrap().0, "3.12.7");
        // `3.x` is the newest stable 3: a release candidate is not a release.
        assert_eq!(resolve("3.x").unwrap().0, "3.13.0");
        assert_eq!(resolve("3").unwrap().0, "3.13.0");
        assert_eq!(
            pick("3.12", &manifest(), "darwin", "arm64").unwrap().1,
            "https://example.invalid/releases/python-3.12.7-darwin-arm64.tar.gz"
        );
        assert_eq!(
            pick("3.12", &manifest(), "linux", "arm64").unwrap().1,
            "https://example.invalid/releases/python-3.12.7-linux-22.04-arm64.tar.gz"
        );
        for request in [">=3.10", "^3.12", "~3.11"] {
            let reason = resolve(request).unwrap_err();
            assert!(reason.contains("not resolved"), "{reason}");
            assert!(reason.contains(request), "{reason}");
        }
        assert_eq!(
            resolve("3.9").unwrap_err(),
            "no stable Python release matches `3.9`"
        );
        assert_eq!(
            resolve("3.10.1").unwrap_err(),
            "no Python 3.10.1 archive for linux x64"
        );
    }

    #[test]
    fn the_release_hash_names_its_file() {
        let sums = "5DD00D3C42D786C6F531CD2ACEB8760175CF6A7DFFCF0CCC5E0A58B26404E70D python-3.12.7-darwin-arm64.tar.gz\n\
                    ECBB4D2161F0D77D75B6A5D0CE179AFCD449478A163B2120F034F6345C9F81F6 python-3.12.7-linux-20.04-x64.tar.gz\n";
        assert_eq!(
            sha256_of(sums, "python-3.12.7-linux-20.04-x64.tar.gz").unwrap(),
            "ECBB4D2161F0D77D75B6A5D0CE179AFCD449478A163B2120F034F6345C9F81F6"
        );
        assert!(sha256_of(sums, "python-3.12.7-linux-24.04-x64.tar.gz").is_none());
    }

    #[test]
    fn activation_needs_bin_python3() {
        let dir = std::env::temp_dir().join(format!("localci-python-{}", crate::install::unique()));
        let bin = dir.join("python/3.12.7/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("python3"), "#!/bin/sh\n").unwrap();
        let need = |request: &str, exact: bool| Need {
            leg: "build".into(),
            step: 1,
            action: "actions/setup-python@v6".into(),
            tool: Some(Tool::Python),
            request: request.into(),
            source: "with.python-version".into(),
            exact,
            policy: None,
            components: vec![],
            targets: vec![],
            note: None,
            env: Default::default(),
        };
        let a = activation(&need("3.12.7", true), &dir).unwrap();
        assert_eq!(a.version, "3.12.7");
        assert_eq!(a.paths, vec![dir.join("python").join("3.12.7").join("bin")]);
        if cfg!(target_os = "linux") {
            assert_eq!(
                a.env,
                vec![(
                    "LD_LIBRARY_PATH".to_string(),
                    dir.join("python")
                        .join("3.12.7")
                        .join("lib")
                        .display()
                        .to_string()
                )]
            );
        } else {
            assert!(a.env.is_empty());
        }
        // A request that is not exact needs the version that setup remembered.
        assert!(activation(&need("3.12", false), &dir).is_none());
        fs::write(dir.join("requests.json"), "{ \"python 3.12\": \"3.12.7\" }").unwrap();
        assert_eq!(
            activation(&need("3.12", false), &dir).unwrap().version,
            "3.12.7"
        );
        assert!(activation(&need("3.12.6", true), &dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
