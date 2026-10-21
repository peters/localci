# Code review instructions

These rules set the scope of the Claude review in this repository
(`.github/workflows/claude-review.yml`). `AGENTS.md` has the full project
rules.

## What to report

- Report a finding only when you can name a concrete input or state and the
  wrong result that it causes.
- A PR that lacks an item that `AGENTS.md` or this file requires is a finding
  without a runtime input.
- Report findings in the lines that this PR changes.
- In unchanged code, report only a problem that this PR causes, makes
  reachable or makes worse.
- In round 2, start with the lines that changed since round 1. In the other
  lines of this PR, report a missed defect only with a concrete trigger. Do not
  look there for new attack surfaces.
- Do not report a design decision that a code comment or a document on the
  base branch states as deliberate. For a PR from the owner, the PR body and
  the documents of this PR count too.
- Do not report a finding again when a reply from the owner or a collaborator
  on its thread gave the reason that it is not a defect. Report it again only
  when the code changes.
- Do not report style, naming or formatting.

## Rules for this repository

Read `docs/adr/README.md` and each record in `docs/adr/` that the change
touches.

- A change that sets how localci behaves for a workflow feature needs a new or
  changed record in `docs/adr/`. A fix that makes localci do what GitHub does
  needs a case folder and a row in `docs/compatibility.md`
  (`docs/procedures/add-a-regression-case.md`).
- No content from private workflows: names, commands, image names, hosts or
  values (ADR-0006). Public projects are not private.
- Each source file stays below 600 lines.
- Documents in `docs/` use ASD-STE100 Simplified Technical English. No em
  dashes in any text.
- A new dependency needs a reason in the commit message.
- The skill in `skills/localci/` is for users of any repository. It must not
  name private repositories or hosts.
