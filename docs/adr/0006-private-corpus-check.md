# ADR-0006: Examine private workflows outside the repository

Status: Accepted.
Date: 2026-10-10.

## Context

localci must work with the workflows of many private repositories. Their
content is secret. It must not go into the localci repository, its tests or
its issues.

## Decision

- `localci check PATH` loads each workflow below PATH. It expands each matrix.
  It evaluates each expression and each `if:`. It shows an error for each
  part that localci cannot run correctly.
- The `tests/corpus.rs` test does the same check for the paths in the
  `LOCALCI_CORPUS` variable. Without the variable, the test does nothing.
- If the check finds a bug, write a new workflow that shows only the problem.
  Do not copy names, commands or values from the private workflow.

## Consequences

- The private workflows stay on the computer that has access to them.
- Each bug from a private workflow becomes a public case folder. The case
  folder keeps the bug fixed for all users.
