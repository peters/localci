//! A job's `services:` as Docker containers on this host, removed when the leg ends.

use crate::expr::{self, Ctx};
use crate::workflow::{to_json, yaml_text};
use serde_json::{json, Map, Value};
use serde_yaml::Value as Yaml;
use std::{
    process::Command,
    time::{Duration, Instant},
};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(180);

/// The running service containers; dropping it removes them.
pub struct Services {
    names: Vec<String>,
}

impl Drop for Services {
    fn drop(&mut self) {
        if !self.names.is_empty() {
            let _ = Command::new(docker())
                .args(["rm", "-f", "-v"])
                .args(&self.names)
                .output();
        }
    }
}

fn docker() -> String {
    std::env::var("LOCALCI_DOCKER").unwrap_or_else(|_| "docker".into())
}

fn output(args: &[&str]) -> Result<String, String> {
    let out = Command::new(docker())
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", docker()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `docker run` arguments for one service definition.
pub fn run_args(container: &str, def: &Yaml, ctx: &Ctx) -> Result<Vec<String>, String> {
    let text = |y: &Yaml| {
        let t = yaml_text(y);
        expr::interpolate(&t, ctx)
    };
    let image = def
        .get("image")
        .map(text)
        .transpose()?
        .filter(|i| !i.is_empty())
        .ok_or("a service needs an image")?;
    let mut args: Vec<String> = ["run", "-d", "--name", container]
        .map(String::from)
        .to_vec();
    if let Some(Yaml::Mapping(env)) = def.get("env") {
        for (k, v) in env {
            args.extend(["-e".into(), format!("{}={}", yaml_text(k), text(v)?)]);
        }
    }
    if let Some(Yaml::Sequence(ports)) = def.get("ports") {
        for port in ports {
            args.extend(["-p".into(), text(port)?]);
        }
    }
    if let Some(options) = def.get("options") {
        args.extend(crate::shell::split(&text(options)?));
    }
    args.push(image);
    Ok(args)
}

pub fn start(defs: &Yaml, ctx: &Ctx, quiet: bool) -> Result<(Services, Value), String> {
    let mut services = Services { names: Vec::new() };
    let mut context = Map::new();
    let Yaml::Mapping(defs) = defs else {
        return Ok((services, Value::Object(context)));
    };
    for (name, def) in defs {
        let name = yaml_text(name);
        let container = format!("localci-{}-{name}", std::process::id());
        let args = run_args(&container, def, ctx)?;
        if !quiet {
            eprintln!("── service {name}: {} {}", docker(), args.join(" "));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // Named before it starts: a failed `docker run` can leave a created container.
        services.names.push(container.clone());
        let id = output(&refs).map_err(|e| {
            let hint = if e.contains("already allocated") || e.contains("address already in use") {
                "\nA local process already uses this port. Stop it, or add --no-services to use it as the service."
            } else {
                ""
            };
            format!("service {name} did not start: {e}{hint}")
        })?;
        wait_healthy(&container).map_err(|e| format!("service {name}: {e}"))?;
        let mut ports = Map::new();
        if let Some(Value::Array(list)) = def.get("ports").map(to_json) {
            for port in list {
                let spec = expr::to_text(&port);
                let inner = spec
                    .rsplit(':')
                    .next()
                    .unwrap_or(&spec)
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let mapped = output(&["port", &container, &inner]).unwrap_or_default();
                let host_port = mapped
                    .lines()
                    .next()
                    .and_then(|l| l.rsplit(':').next())
                    .unwrap_or_default();
                ports.insert(inner, Value::String(host_port.to_string()));
            }
        }
        context.insert(name, json!({ "id": id, "ports": ports, "network": "" }));
    }
    Ok((services, Value::Object(context)))
}

fn wait_healthy(container: &str) -> Result<(), String> {
    let format = "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}";
    let started = Instant::now();
    loop {
        match output(&["inspect", "-f", format, container])?.as_str() {
            "healthy" | "running" => return Ok(()),
            "unhealthy" | "exited" | "dead" => {
                let logs = output(&["logs", "--tail", "20", container]).unwrap_or_default();
                return Err(format!("container is not healthy\n{logs}"));
            }
            _ if started.elapsed() > HEALTH_TIMEOUT => return Err("not healthy after 180s".into()),
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Status;

    #[test]
    fn service_definition_to_docker_args() {
        let def: Yaml = serde_yaml::from_str(
            "image: db:1\nenv:\n  USER: me\nports:\n  - 5432:5432\noptions: >-\n  --health-cmd \"ready -U me\"\n  --health-interval 5s\n",
        )
        .unwrap();
        let contexts = Map::new();
        let args = run_args(
            "c",
            &def,
            &Ctx {
                contexts: &contexts,
                status: Status::Success,
            },
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "run",
                "-d",
                "--name",
                "c",
                "-e",
                "USER=me",
                "-p",
                "5432:5432",
                "--health-cmd",
                "ready -U me",
                "--health-interval",
                "5s",
                "db:1"
            ]
        );
    }
}
