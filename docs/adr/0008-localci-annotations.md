# ADR-0008: Mark local skips in the workflow with LOCALCI

Status: Accepted.
Date: 2026-10-10.

## Context

Some steps must not run on a developer computer. Examples are steps that
install packages with `sudo` and jobs that deploy. The `--skip` option works
for one run only. A second file with skip rules is one more format to learn
and to keep current.

GitHub rejects unknown keys in a workflow, for example `x-localci: skip`.
YAML comments are not available after the parse. GitHub does not allow the
`env` context in a job-level `if:`.

## Decision

localci sets `LOCALCI` to `true` in two contexts:

- `vars.LOCALCI`, for each `if:`.
- `env.LOCALCI`, for each step. The step processes also get `LOCALCI=true`.

On GitHub, both values are empty. Thus a workflow marks a local skip with a
normal condition:

- For a job: `if: ${{ !vars.LOCALCI }}`.
- For a step: `if: ${{ !env.LOCALCI }}` or `if: ${{ !vars.LOCALCI }}`.

`localci plan` evaluates these conditions before a run. It shows the job or
the step as skipped.

## Consequences

- No second file. The workflow stays the only source.
- GitHub runs the job or step as before.
- If a repository defines a variable with the name `LOCALCI`, the condition
  is also false on GitHub. Do not define that variable.
