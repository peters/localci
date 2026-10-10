# ADR-0005: Let an agent and the user select the subset

Status: Accepted.
Date: 2026-10-10.

## Context

The full CI is slow. Before a push, a smaller set of jobs and steps gives
enough confidence. The correct set depends on the change. An agent can read
the diff and propose the set. The user decides.

## Decision

- `localci plan --json` shows each leg, each step, the step kind and the
  reason that a step does not run. It does not run commands.
- A step kind is `run`, `uses` or `setup`. A `setup` step installs packages or
  tools, for example with `apt-get` or `brew install`.
- The selection options are targets, `--matrix`, `--skip`, `--only`,
  `--skip-setup` and `--ignore-if`. `--skip LEG:N` selects one step.
- `localci run --json -q` writes a report with the result of each step and
  the path of its log. It shows the end of the log of a step that failed.
- The skill in `skills/localci/SKILL.md` tells the agent to propose a
  selection and to ask the user before a long run.

## Consequences

- The agent does not need to read the full log. It reads the report and the
  log of the step that failed.
- A green local run is not the GitHub result. GitHub CI stays the final gate.
