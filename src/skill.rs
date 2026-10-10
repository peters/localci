//! `localci skill`: put the agent skill where coding agents look for skills.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

pub const SKILL: &str = include_str!("../skills/localci/SKILL.md");

/// The agent homes that read `<home>/skills/<name>/SKILL.md`.
const AGENTS: &[(&str, &str)] = &[
    (".claude", "Claude Code"),
    (".codex", "Codex"),
    (".agents", "agents that read ~/.agents"),
];

const HELP: &str = "localci skill: teach your coding agent to use localci

USAGE:
  localci skill install [--dir DIR]   write the skill for every agent found in your home folder
                                      (~/.claude, ~/.codex, ~/.agents), or into DIR/localci
  localci skill show                  print the skill
";

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn write(skills: &Path) -> Result<PathBuf, String> {
    let dir = skills.join("localci");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("SKILL.md");
    std::fs::write(&file, SKILL).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

pub fn main(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("show") => {
            print!("{SKILL}");
            Ok(ExitCode::SUCCESS)
        }
        Some("install") => {
            let mut written = Vec::new();
            if let Some(i) = args.iter().position(|a| a == "--dir") {
                let dir = args.get(i + 1).ok_or("--dir needs a value")?;
                written.push((write(Path::new(dir))?, "the folder you named"));
            } else {
                let home = home().ok_or("cannot find the home folder; use --dir")?;
                for (folder, agent) in AGENTS {
                    if home.join(folder).is_dir() {
                        written.push((write(&home.join(folder).join("skills"))?, *agent));
                    }
                }
                if written.is_empty() {
                    written.push((
                        write(&home.join(".agents/skills"))?,
                        "agents that read ~/.agents",
                    ));
                }
            }
            for (file, agent) in &written {
                println!("installed {} for {agent}", file.display());
            }
            println!("\nYour agent now runs CI locally when you ask, for example: \"run CI locally before you push\".");
            println!("Run `localci skill install` again after an upgrade to refresh the skill.");
            Ok(ExitCode::SUCCESS)
        }
        _ => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
    }
}
