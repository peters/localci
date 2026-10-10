# ADR-0001: Run the steps on the host, in the checkout

Status: Accepted.
Date: 2026-10-10.

## Context

GitHub Actions can be slow or not available. A change then waits a long time
for CI results. Most CI steps are shell commands, for example `cargo test`.
These commands run equally well on a developer computer. A container per job
makes each run slow, because the build cache starts empty.

## Decision

localci runs each `run:` step as a process on the host. The working directory
is the root of the current Git checkout. localci runs a job only when its
`runs-on` labels name the host system: Linux, macOS or Windows. Labels that
start with `ubuntu` or equal `linux` are Linux. Labels that start with `macos`
are macOS. Labels that start with `windows` are Windows.

localci runs the jobs one after the other, in the order of `needs`. The jobs
use the same build cache, for example the Cargo `target` folder.

localci uses the shell commands of GitHub-hosted runners. It writes the
`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH` and `GITHUB_STEP_SUMMARY` files.
It sets `CI=true` and `LOCALCI=true`. It does not set `GITHUB_ACTIONS`, because
some scripts use that variable to find a disposable runner.

## Consequences

- A second run is fast, because the build cache stays.
- A job for another system does not run. Run localci on a computer with that
  system, for example a Mac over SSH.
- The host must have the tools that the setup actions install on GitHub.
- A step can change files in the checkout. Use a separate worktree if that is
  a problem.
- localci ignores `container:`. The steps of that job run on the host.
