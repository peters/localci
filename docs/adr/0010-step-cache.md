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
- The key of a record contains these items:
  - the script and the working directory, after expression evaluation;
  - the shell, the environment and the `PATH` of the step;
  - the system and the checkout folder;
  - the version of each tool that the step runs. The key
  uses FNV-1a, a hash that does not change between Rust releases.
- localci uses a record when the snapshot is the same. For a step with only
  known tool commands (ADR-0009), localci also uses a record when no changed
  file is an input of its tools.
- A step that localci does not run gets the outputs, the environment additions
  and the path additions of the record.

The records are in `$XDG_CACHE_HOME/localci`, or in `~/.cache/localci`. Only
the user can read them, because they can contain `GITHUB_ENV` values.

## Consequences

- The snapshot does not contain the files that `.gitignore` excludes. A step
  that reads an ignored file, for example `.env.local` or generated code, can
  get an old result.
- localci does not make the files again that a step made the first time, for
  example a build folder. If you delete these files, run without `--cache`.
- A step that reads data from outside the checkout, for example from the
  network, can show an old result. Do not use `--cache` for that kind of check.
- The snapshot puts each untracked file that `.gitignore` does not exclude into
  `.git/objects`, large or secret files included. Add such files to
  `.gitignore`, or do not use `--cache`.
- localci writes each record to a temporary file and then renames it, so a
  parallel leg never reads half a record.
- Git keeps the snapshot trees as unreferenced objects. `git gc` removes them
  after the usual expiry time. localci then runs the step again.
