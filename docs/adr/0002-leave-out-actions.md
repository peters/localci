# ADR-0002: Leave out `uses:` actions

Status: Accepted.
Date: 2026-10-10.

## Context

Most actions prepare a disposable runner. Examples are `actions/checkout`,
`actions/setup-node` and `Swatinem/rust-cache`. A developer computer has a
checkout and the tools already. An action runtime also needs Node.js, network
access and the action source.

## Decision

localci does not run a `uses:` step. The plan and the report show the step as
`skipped` with the reason `uses: actions do not run locally`. A job that calls a
reusable workflow does not run either.

## Consequences

- localci needs no network access to start.
- An output of a skipped action is empty. An expression that reads it gets an
  empty string.
- If a later step needs a tool that an action installs, install the tool on the
  host.
