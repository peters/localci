# ADR-0009: Leave out tool steps whose inputs did not change

Status: Accepted.
Date: 2026-10-10.

## Context

Most of the time of a local run goes to build and test steps, for example
`cargo test`. A change to documentation or to a script does not change their
result. A wrong guess must not hide a failure.

## Decision

`localci run --affected` compares the checkout with a base commit. The changed
files are the committed, staged, unstaged and untracked files.

localci leaves out a step only when all these conditions are true:

1. Each command in the step is a known tool command, a shell builtin, or a
   function of the same script. Known tools are Cargo, npm and other Node
   tools, dotnet, Go, Gradle and Swift. A step can call a script in the
   repository. localci then examines the script in the same way. The script
   path starts at the working directory of the step and must stay in the
   repository.
2. The step does not use `$(...)`, backticks or process substitution.
3. The step has no `id`, and it does not write a `GITHUB_ENV`, `GITHUB_PATH`,
   `GITHUB_OUTPUT` or `GITHUB_STATE` file. Later steps can read these values.
4. These parts are the same as in the base:
   - the step and each step before it, because an earlier step can change
     the environment of a later step;
   - the settings of its job, for example `env`, `runs-on` and the matrix;
   - the `env` and `defaults` of the workflow.
5. No changed file is an input of the tools of the step. These files are inputs:
   - a source file or a configuration file of the tool;
   - a file in a project folder of the tool;
   - a file whose path a source file names. A project
   folder is, for example, a Rust crate with `[package]` in `Cargo.toml`. Tests
   can read any file in it. An example of a named file is a file that
   `include_str!` reads.
6. No script that the step calls changed.
7. The working directory of the step has no expression, and the step does not
   read `needs` outputs. A local action (`uses: ./...`) before the step did
   not change.

The base is `--base REF`, or the merge base with the default branch of
`origin`. If neither exists, localci uses the parent commit and shows a
warning.

Any other step runs. The plan and the report show the reason for each step that
localci leaves out.

## Consequences

- A change to documentation runs the repository checks only. A change to Rust
  source runs all Rust steps.
- The check of file names is wide. A Rust file that names a file in a comment
  also makes that file an input. This makes more steps run, not fewer.
- Files that `.gitignore` excludes are not changed files. A step that reads
  an ignored file does not run because of it.
- A test can read a file outside its project folder without naming it, for
  example a file in the repository root. localci cannot see that read. Run
  without `--affected` before the final push.
- The plan shows the reason for each tool step that runs.
