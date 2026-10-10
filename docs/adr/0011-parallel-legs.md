# ADR-0011: Run independent legs at the same time

Status: Accepted.
Date: 2026-10-10.

## Context

GitHub runs the jobs of a workflow on different runners at the same time.
localci runs them one after the other by default. A computer with many cores
can do more work at the same time.

## Decision

`localci run --parallel N` runs a maximum of N legs at the same time. A leg
starts when all picked legs of the jobs in its `needs` are complete.
`--parallel 0` uses one leg for each four CPU cores, with a minimum of two and
a maximum of four.

A matrix job counts as failed in `needs` when one of its legs failed, also when
another leg passes later. If a leg stops with an internal error, it counts as
failed, and the other legs continue.

With more than one leg, localci does not show the output of the steps. It shows
the step names and the end of the log of a step that failed.

`--fast` is `--affected --cache --parallel 0`.

## Consequences

- Build tools can wait for each other. For example, Cargo locks the build
  folder, so two Cargo steps in the same checkout do not compile at the same
  time.
- Two legs can use the same service port. Then one of them fails. Use
  `--parallel 1` for these workflows.
