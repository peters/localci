# ADR-0010: Use the result of a step that passed on the same inputs

Status: Accepted.
Date: 2026-10-10.

## Context

During a review round, most changes are small. A second local run repeats many
steps that passed with the same inputs a few minutes before.

## Decision

`localci run --cache` keeps a record of each step that passed.

- At the start of a run, localci writes a snapshot of the checkout as a Git
  tree object. It uses a temporary index, so the real index does not change.
  The snapshot includes untracked files and ignores the files in `.gitignore`.
- The key of a record contains the script after expression evaluation, the
  shell, the working directory, the environment of the step and the system.
- localci uses a record when the snapshot is the same. For a step with only
  known tool commands (ADR-0009), localci also uses a record when no changed
  file between the two snapshots is an input of its tools.
- A step that localci does not run gets the outputs, the environment additions
  and the path additions of the record.

The records are in `$XDG_CACHE_HOME/localci`, or in `~/.cache/localci`.

## Consequences

- A step that reads data from outside the checkout, for example from the
  network, can show an old result. Do not use `--cache` for that kind of check.
- Git keeps the snapshot trees as unreferenced objects. `git gc` removes them
  after the usual expiry time. localci then runs the step again.
