# Architecture decision records

This folder keeps the decisions that control what localci does. Each record
uses ASD-STE100 Simplified Technical English (STE).

## Records

| Number | Decision | Status |
|---|---|---|
| [ADR-0001](0001-run-steps-on-the-host.md) | Run the steps on the host, in the checkout | Accepted |
| [ADR-0002](0002-leave-out-actions.md) | Leave out `uses:` actions | Accepted |
| [ADR-0003](0003-expression-subset.md) | Evaluate a subset of the expression language | Accepted |
| [ADR-0004](0004-services-as-docker-containers.md) | Start `services:` as Docker containers | Accepted |
| [ADR-0005](0005-agent-picks-the-subset.md) | Let an agent and the user select the subset | Accepted |
| [ADR-0006](0006-private-corpus-check.md) | Examine private workflows outside the repository | Accepted |
| [ADR-0007](0007-case-folders-for-regressions.md) | Record each bug as a case folder | Accepted |
| [ADR-0008](0008-localci-annotations.md) | Mark local skips in the workflow with LOCALCI | Accepted |
| [ADR-0009](0009-affected-steps.md) | Leave out tool steps whose inputs did not change | Accepted |
| [ADR-0010](0010-step-cache.md) | Use the result of a step that passed on the same inputs | Accepted |
| [ADR-0011](0011-parallel-legs.md) | Run independent legs at the same time | Accepted |
| [ADR-0012](0012-setup-steps.md) | Install the toolchains that setup actions declare | Accepted |

## When to write a new record

Write a new record when a change sets how localci behaves for a workflow
feature. Do not write a record for a fix that makes localci do what GitHub does.
Add a case folder and a row in [the compatibility table](../compatibility.md)
for that fix. The [regression procedure](../procedures/add-a-regression-case.md)
gives the steps.

## Template

Copy this template to `NNNN-short-name.md`. Use the next free number.

```markdown
# ADR-NNNN: Short title

Status: Proposed | Accepted | Replaced by ADR-NNNN.
Date: YYYY-MM-DD.

## Context

The problem. The facts that apply. Links to the cases that show the problem.

## Decision

What localci does.

## Consequences

What becomes easier. What becomes more difficult. What localci does not do.
```
