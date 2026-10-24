//! Java: the Temurin release metadata, the archive and its activation (ADR-0012).
//!
//! Only the Temurin distribution of Eclipse Adoptium installs. The pick function
//! takes parsed metadata, so tests need no network. The bytes of the archive are
//! checked against the SHA-256 of the metadata in memory, and the same bytes are
//! unpacked.

use crate::install::{
    archive, fetch_json, platform, remembered, safe_name, Activation, Installed, Platform, Sum,
};
use crate::install_sources::numbers;
use crate::toolchain::{Need, Tool};
use serde_json::Value;
use std::path::Path;

/// The GA releases of one feature release, for one architecture and system.
const METADATA: &str = "https://api.adoptium.net/v3/assets/feature_releases";

/// The version of a Temurin build without the build information: `21.0.13+9.0.LTS`
/// is `21.0.13`.
fn bare(text: &str) -> &str {
    text.trim().split('+').next().unwrap_or_default()
}

/// The text of a request without a `v` prefix. A request is a version, so it has no
/// build information.
fn requested(text: &str) -> &str {
    text.trim().trim_start_matches('v')
}

/// The numbers of a release, with the build as more parts, so `21.0.12+101` comes
/// after `21.0.12+8`.
fn order(semver: &str) -> Vec<u64> {
    numbers(&semver.replace('+', "."))
}

/// The request as the version that it names. A request that is not a version, for
/// example a range, is refused with its reason (ADR-0012 decision 4).
fn wanted(request: &str) -> Result<&str, String> {
    let text = requested(request);
    let is_version = !text.is_empty()
        && text
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !is_version {
        return Err(format!(
            "`{request}` is not a Java version; write one version such as `21` or `21.0.5`"
        ));
    }
    Ok(text)
}

/// The architecture as the Adoptium API names it.
fn java_arch(arch: &str) -> Result<&'static str, String> {
    match arch {
        "x64" => Ok("x64"),
        "arm64" => Ok("aarch64"),
        other => Err(format!(
            "Temurin has no JDK for the architecture `{other}`; localci installs x64 and arm64"
        )),
    }
}

/// The system as the Adoptium API names it.
fn java_os(os: &str) -> Result<&'static str, String> {
    match os {
        "linux" => Ok("linux"),
        "darwin" => Ok("mac"),
        other => Err(format!(
            "Temurin has no JDK for the system `{other}`; localci installs on Linux and macOS"
        )),
    }
}

/// The JDK binary of `release` for `os` and `arch`. A release can hold the JDK and
/// the JRE, and the normal and the large heap size.
fn binary<'a>(release: &'a Value, os: &str, arch: &str) -> Option<&'a Value> {
    let ok = |b: &&Value| {
        b["os"].as_str() == Some(os)
            && b["architecture"].as_str() == Some(arch)
            && b["image_type"].as_str() == Some("jdk")
    };
    let found: Vec<&Value> = release["binaries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(ok)
        .collect();
    found
        .iter()
        .find(|b| b["heap_size"].as_str() == Some("normal"))
        .or_else(|| found.first())
        .copied()
}

/// The newest GA release that matches `request`, and the name and SHA-256 of its
/// archive for `os` and `arch` (`linux`, `mac`, `x64`, `aarch64`). A request is a
/// feature release such as `21`, which is its newest GA, or one build such as
/// `21.0.5`.
pub fn pick(
    request: &str,
    releases: &Value,
    os: &str,
    arch: &str,
) -> Result<(String, String, String), String> {
    let want = wanted(request)?;
    let prefix = format!("{want}.");
    let matched = |version: &str| version == want || version.starts_with(&prefix);
    let mut found = false;
    let mut best: Option<(Vec<u64>, String, String, String)> = None;
    for release in releases.as_array().into_iter().flatten() {
        let semver = release["version_data"]["semver"]
            .as_str()
            .unwrap_or_default();
        if !matched(bare(semver)) {
            continue;
        }
        found = true;
        let Some(package) = binary(release, os, arch).map(|b| &b["package"]) else {
            continue;
        };
        let (Some(link), Some(hash)) = (package["link"].as_str(), package["checksum"].as_str())
        else {
            continue;
        };
        let entry = (
            order(semver),
            bare(semver).to_string(),
            link.to_string(),
            hash.to_string(),
        );
        if best.as_ref().is_none_or(|b| b.0 < entry.0) {
            best = Some(entry);
        }
    }
    match best {
        Some((_, version, url, hash)) => Ok((version, url, hash)),
        None if found => Err(format!("no Temurin JDK {want} for {os}-{arch}")),
        None => Err(format!("no Temurin JDK GA matches `{request}`")),
    }
}

/// The version, URL and SHA-256 of `request`, from the official metadata.
pub fn release(request: &str, p: &Platform) -> Result<(String, String, String), String> {
    let (os, arch) = (java_os(p.os)?, java_arch(p.arch)?);
    let number = wanted(request)?
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string();
    let url =
        format!("{METADATA}/{number}/ga?architecture={arch}&image_type=jdk&os={os}&vendor=eclipse");
    pick(request, &fetch_json(&url)?, os, arch)
}

/// The distribution of the policy, when localci does not install it: only Temurin
/// does (ADR-0012). `policy` is the `policy` of the need, for example
/// `distribution: temurin`.
pub fn distribution(policy: Option<&str>) -> Option<String> {
    let named = policy
        .and_then(|p| p.strip_prefix("distribution: "))
        .map(str::trim)
        .unwrap_or("(none)");
    if named.eq_ignore_ascii_case("temurin") {
        return None;
    }
    Some(format!(
        "the distribution `{named}` is not installed; only the Temurin distribution installs"
    ))
}

/// Installs the Temurin JDK of `need` into `<dir>/java/<version>`, which holds the
/// top folder of the archive, so `bin/java` is at its top. The version is the
/// version of the release without its build. When the metadata cannot be reached,
/// the installed version of the request is used (decision 8).
pub fn install(need: &Need, dir: &Path) -> Result<Installed, String> {
    if let Some(why) = distribution(need.policy.as_deref()) {
        return Err(why);
    }
    let (version, url, hash) = match release(&need.request, &platform()?) {
        Ok(found) => found,
        Err(reason) => {
            let a = activation(need, dir).ok_or(reason)?;
            return Ok(Installed {
                path: dir.join("java").join(&a.version),
                version: a.version,
                fresh: false,
            });
        }
    };
    let target = dir.join("java").join(safe_name(&version)?);
    let fresh = archive(&target, &url, Sum::Sha256, &hash, true)?;
    Ok(Installed {
        version,
        path: target,
        fresh,
    })
}

/// `PATH` gets `<dir>/java/<version>/bin` and the step gets `JAVA_HOME`, so a build
/// finds its JDK too. `None` until `bin/java` is there.
pub fn activation(need: &Need, dir: &Path) -> Option<Activation> {
    let version = if need.exact {
        safe_name(requested(&need.request)).ok()?.to_string()
    } else {
        remembered(dir, Tool::Java, &need.request)?
    };
    let root = dir.join("java").join(&version);
    if !root.join("bin/java").is_file() {
        return None;
    }
    Some(Activation {
        version,
        paths: vec![root.join("bin")],
        env: vec![("JAVA_HOME".to_string(), root.display().to_string())],
        label: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(os: &str, arch: &str, kind: &str, heap: &str, hash: &str) -> Value {
        serde_json::json!({
            "os": os, "architecture": arch, "image_type": kind, "heap_size": heap,
            "package": {
                "name": format!("OpenJDK-{kind}_{arch}_{os}_{heap}.tar.gz"),
                "link": format!("https://example.test/{arch}/{os}/{heap}.tar.gz"),
                "checksum": hash}})
    }

    fn release(semver: &str, hash: &str) -> Value {
        serde_json::json!({
            "version_data": {"semver": semver},
            "binaries": [
                binary("linux", "x64", "jre", "normal", "jre"),
                binary("linux", "x64", "jdk", "large", "large"),
                binary("linux", "x64", "jdk", "normal", hash),
                binary("linux", "aarch64", "jdk", "normal", "arm"),
                binary("mac", "aarch64", "jdk", "normal", "mac")]})
    }

    fn index() -> Value {
        serde_json::json!([
            release("21.0.13+9.0.LTS", "newest"),
            release("21.0.12+8.0.LTS", "old"),
            release("17.0.13+11", "lts17")
        ])
    }

    #[test]
    fn java_requests() {
        let pick = |request: &str, os: &str, arch: &str| pick(request, &index(), os, arch);
        // One build, and the normal heap size of the JDK, not the JRE or the large one.
        assert_eq!(
            pick("21.0.13", "linux", "x64").unwrap(),
            (
                "21.0.13".into(),
                "https://example.test/x64/linux/normal.tar.gz".into(),
                "newest".into()
            )
        );
        // A feature release is its newest GA.
        assert_eq!(pick("21", "linux", "x64").unwrap().0, "21.0.13");
        assert_eq!(pick("17", "linux", "x64").unwrap().0, "17.0.13");
        // The build information of the metadata does not become part of the version.
        assert_eq!(pick("21.0.12", "linux", "x64").unwrap().2, "old");
        // A request that is not a version stays a request, with the reason.
        for text in [">=21", "^21", "21.x", "latest", "21.0.5+9"] {
            assert!(
                pick(text, "linux", "x64")
                    .unwrap_err()
                    .contains("is not a Java version"),
                "{text}"
            );
        }
        assert!(pick("21.0.5", "linux", "x64")
            .unwrap_err()
            .contains("no Temurin JDK GA matches"));
        // The archives of the other systems and architectures.
        assert_eq!(pick("21", "linux", "aarch64").unwrap().2, "arm");
        assert_eq!(pick("21", "mac", "aarch64").unwrap().2, "mac");
        assert!(pick("21", "mac", "x64")
            .unwrap_err()
            .contains("no Temurin JDK 21 for mac-x64"));
        assert_eq!(java_arch("x64").unwrap(), "x64");
        assert_eq!(java_arch("arm64").unwrap(), "aarch64");
        assert!(java_arch("x86").unwrap_err().contains("no JDK"));
        assert_eq!(java_os("darwin").unwrap(), "mac");
        assert!(java_os("windows").unwrap_err().contains("no JDK"));
    }

    #[test]
    fn the_newest_ga_of_a_feature_release_wins() {
        let index = serde_json::json!([
            release("21.0.12+101.0.LTS", "rebuilt"),
            release("21.0.12+8.0.LTS", "older")
        ]);
        assert_eq!(pick("21", &index, "linux", "x64").unwrap().2, "rebuilt");
        let index = serde_json::json!([
            release("21.0.12+8.0.LTS", "older"),
            release("21.0.12+101.0.LTS", "rebuilt")
        ]);
        assert_eq!(pick("21", &index, "linux", "x64").unwrap().2, "rebuilt");
    }

    #[test]
    fn only_temurin_installs() {
        assert_eq!(distribution(Some("distribution: temurin")), None);
        assert_eq!(distribution(Some("distribution: Temurin")), None);
        let other = distribution(Some("distribution: zulu")).unwrap();
        assert!(
            other.contains("only the Temurin distribution installs"),
            "{other}"
        );
        assert!(other.contains("zulu"));
        assert!(distribution(None)
            .unwrap()
            .contains("only the Temurin distribution installs"));
    }
}
