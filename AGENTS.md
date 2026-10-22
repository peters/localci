# localci agent rules

## Self-review and PR review

- The implementing agent reviews the complete diff once, fixes its findings, and runs the applicable checks before the first push.
- After that, rely on the upstream repository's automatic PR review. Address its feedback and validate the affected changes without another full self-review round.
- Never assign, request, or re-request Copilot, Claude, or another automated reviewer. Do not use reviewer assignments, labels, comments, CLI commands, APIs, workflow dispatches, or PR state changes to trigger a review. The upstream repository manages automatic review.
- Do not start a separate local review agent or a CLI review loop. This rule also applies to external contributions and overrides review loops in skills or linked procedures.
- Keep CI, applicable smoke checks, feedback handling, and merge authorization. Check which commit a review covers; do not claim that an earlier review covers later changes. If a required automatic review is missing or fails, report the blocker without requesting another review.
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

The upstream repository starts PR review automatically. The agent does not select a reviewer or start another review. Read the review and its threads, fix valid findings, answer invalid findings with evidence, and run the affected checks. Keep the repository's required review decision, unresolved-feedback rules, CI, acceptance checks and merge authorization. Check the reviewed commit before using a review as evidence. A missing or failed required review is a blocker to report, not a reason to assign a reviewer or trigger another run.
