# ADR-0007: Record each bug as a case folder

Status: Accepted.
Date: 2026-10-10.

## Context

Each new workflow can show a new bug. A fix must stay fixed. Adding a test
must be quick, or nobody does it.

## Decision

Each folder in `tests/cases/` is one test. It contains two files:

- `workflow.yml` is the workflow to run.
- `case.json` gives the arguments, the exit code and the JSON that the output
  must contain. It can limit the case to some hosts, or require a tool such
  as Docker.

`tests/cases.rs` runs each folder in a new Git repository. A new folder needs
no Rust code. The name of the folder starts with the next free number.

## Consequences

- A bug report becomes a test in two files.
- The `description` in `case.json` tells why the case exists. Write where the
  bug came from, but not private content.
- Each case also gets a row in [the compatibility table](../compatibility.md).
