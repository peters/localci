# ADR-0003: Evaluate a subset of the expression language

Status: Accepted.
Date: 2026-10-10.

## Context

Workflows use `${{ }}` expressions in names, `runs-on`, `env`, `if:` and `run:`.
localci must evaluate them the same way as GitHub, or it runs the wrong steps.

## Decision

`src/expr.rs` evaluates these parts of the language:

- The literals `true`, `false`, `null`, numbers and strings in single quotes.
- The operators `!`, `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`, `.`, `[ ]`
  and the `*` filter.
- The functions `contains`, `startsWith`, `endsWith`, `format`, `join`,
  `toJSON`, `fromJSON`, `hashFiles`, `success`, `failure`, `always` and
  `cancelled`.

The rules for types are the rules of GitHub. A string compare ignores case. A
property that does not exist gives `null`. An `if:` without a status function
gets `success() &&` before it.

The contexts come from the local checkout:

- `github` comes from Git. Off the default branch, the event is
  `pull_request`. The base is the merge base with the default branch.
- `secrets` and `vars` read the environment variable with the same name.
- `hashFiles` gives an empty string, because localci keeps no cache.

## Consequences

- An unknown function is an error. `localci check` shows it before a run.
- To add a function, add it to `call` in `src/expr.rs`. Then add a unit test
  and a row in the compatibility table.
