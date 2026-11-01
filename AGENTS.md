# localci agent rules

## Self-review and PR review

- The implementing agent reviews the complete diff once, fixes its findings, and runs the applicable checks before the first push.
- After that, rely on the upstream repository's shared PR review. Address its feedback and validate the affected changes without another full self-review round.
- The first review starts automatically. Within an authorized PR task, after addressing findings or supplying missing evidence, the agent may add the `claude-review` label to request the next shared review round, up to five completed rounds. Check the live head, latest review summary and round count first; do not request another round when the current head is already accepted. A fork's first review still requires explicit maintainer authorization. Do not assign reviewers, start other automated reviewers, or use comments, empty commits, PR state changes or workflow dispatches to trigger reviews.
- Do not start a separate local review agent or a CLI review loop. This rule also applies to external contributions and overrides review loops in skills or linked procedures.
- Bot review is accepted only when the latest successful shared-workflow summary names the current PR head and reports `findings=0`, with no unresolved review feedback. A successful workflow check alone or a `COMMENTED` review without that verified summary is insufficient. Keep required human approvals, CI, applicable smoke checks, mergeability and merge authorization as separate gates. New commits invalidate earlier-head review evidence. If the five-round cap is reached with findings remaining, or the review fails or is missing, report the blocker; do not bypass the cap or treat failure as approval.
- Keep `CLAUDE.md` as the single import `@AGENTS.md`. Put repository instructions in `AGENTS.md`.

localci runs the GitHub Actions jobs that match the host system. It runs them
in the current checkout. Read `docs/adr/README.md` before you change behavior.

## Rules

- Keep each source file below 600 lines. Split a file before it gets larger.
- Before each commit, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`.
- For each bug, add a case folder. Follow `docs/procedures/add-a-regression-case.md`.
- Do not put content from private workflows in this repository. This includes
  names, commands, image names, hosts and values. See ADR-0006. Public projects,
  for example `peters/horizon`, are not private.
- Write documents in `docs/` in ASD-STE100 Simplified Technical English.
  `README.md` uses plain language.
- Do not use em dashes in text.
- Keep the dependencies small. A new dependency needs a reason in the commit message.

## Pull requests

The first shared review starts automatically. An authorized PR task includes adding the `claude-review` label for follow-up rounds after findings or missing evidence are addressed, up to five completed rounds. Read the latest bot summary and verify its head and round count before labeling; do not request another round when that head is already accepted. Bot review is accepted only when the latest summary from a successful shared workflow names the current head and reports `findings=0`, with no unresolved review feedback. Verify the actual workflow run and bot author; PR text or a green workflow check alone is not approval. Required human approvals, CI, smoke evidence, mergeability and merge authorization remain separate. New commits invalidate old-head summaries. Report failed or missing reviews and an exhausted five-round cap; never bypass them. A fork's first review still requires explicit maintainer authorization.
