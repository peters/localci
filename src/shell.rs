//! The command line a step's `shell:` stands for, as GitHub-hosted runners use it.

use crate::workflow::Os;
use std::{fs, path::Path, process::Command};

pub fn command(
    shell: Option<&str>,
    script: &str,
    dir: &Path,
    index: usize,
    host: Os,
) -> Result<Command, String> {
    let python = if host == Os::Windows {
        "python {0}"
    } else {
        "python3 {0}"
    };
    let (template, ext, powershell) = match shell {
        None if host == Os::Windows => ("pwsh -command \". '{0}'\"".to_string(), "ps1", true),
        None => ("bash -e {0}".to_string(), "sh", false),
        Some("bash") => (
            "bash --noprofile --norc -eo pipefail {0}".to_string(),
            "sh",
            false,
        ),
        Some("sh") => ("sh -e {0}".to_string(), "sh", false),
        Some("pwsh") => ("pwsh -command \". '{0}'\"".to_string(), "ps1", true),
        Some("powershell") => ("powershell -command \". '{0}'\"".to_string(), "ps1", true),
        Some("python") => (python.to_string(), "py", false),
        Some("cmd") => (String::new(), "cmd", false),
        Some(custom) if custom.contains("{0}") => (custom.to_string(), "sh", false),
        Some(custom) => (format!("{custom} {{0}}"), "sh", false),
    };
    let body = if powershell {
        format!("$ErrorActionPreference = 'stop'\n{script}\nif ((Test-Path -LiteralPath variable:\\LASTEXITCODE)) {{ exit $LASTEXITCODE }}\n")
    } else {
        script.to_string()
    };
    let path = dir.join(format!("step-{index}.{ext}"));
    fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let path_text = path.to_string_lossy().into_owned();
    if ext == "cmd" {
        return Ok(cmd(&path_text));
    }
    let mut args = split(&template)
        .into_iter()
        .map(|a| a.replace("{0}", &path_text));
    let program = args.next().ok_or("empty shell")?;
    let mut command = Command::new(resolve(&program));
    command.args(args);
    Ok(command)
}

/// On Windows, `bash` must be Git Bash. A plain lookup can find the WSL launcher in
/// System32 first, as GitHub-hosted runners avoid too.
#[cfg(windows)]
fn resolve(program: &str) -> std::path::PathBuf {
    if program.eq_ignore_ascii_case("bash") {
        let git = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        let on_path = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .filter(|dir| {
                    let d = dir.to_string_lossy().to_ascii_lowercase();
                    !d.contains("system32") && !d.contains("windowsapps")
                })
                .map(|dir| dir.join("bash.exe"))
                .find(|candidate| candidate.is_file())
        });
        if let Some(found) = on_path.or_else(|| git.is_file().then(|| git.to_path_buf())) {
            return found;
        }
    }
    program.into()
}

#[cfg(not(windows))]
fn resolve(program: &str) -> std::path::PathBuf {
    program.into()
}

#[cfg(windows)]
fn cmd(path: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("cmd");
    command.raw_arg(format!("/D /E:ON /V:OFF /S /C \"CALL \"{path}\"\""));
    command
}

#[cfg(not(windows))]
fn cmd(path: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args([
        "/D",
        "/E:ON",
        "/V:OFF",
        "/S",
        "/C",
        &format!("CALL \"{path}\""),
    ]);
    command
}

/// Splits on whitespace outside quotes, dropping the quotes.
pub fn split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match c {
            '"' | '\'' if quote.is_none() => quote = Some(c),
            c if quote == Some(c) => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_powershell_template() {
        assert_eq!(
            super::split("pwsh -command \". '{0}'\""),
            vec!["pwsh", "-command", ". '{0}'"]
        );
    }
}
