# localci agent rules

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

Claude reviews each pull request, at most 2 rounds (the owner's decision,
2026-10-21). Copilot does not review: do not request a Copilot review and do
not wait for one. The `Claude Review` workflow
(`.github/workflows/claude-review.yml`) does the review with the rules in
`.github/review-instructions.md`.

- Round 1 starts when a pull request from a branch of this repository opens,
  reopens or leaves draft.
- Round 2 starts when you add the `claude-review` label:
  `gh pr edit <n> --add-label claude-review`. Push all fixes of round 1 first.
  The workflow removes the label when it finishes; adding a label that is still
  there starts nothing. The workflow refuses a third round.
- A pull request from a fork or a bot gets a round only from the label, after
  you have read its full diff.
- A round is one summary comment from `github-actions[bot]` whose last line is
  `<!-- claude-review head=<sha> findings=<n> -->`. A run that fails posts no
  summary and uses no round: read its log, fix the cause and add the label
  again.
- Review the full diff yourself and run the checks before the pull request
  opens and before round 2. The review on the pull request is the final gate,
  not the place to iterate. Fix or answer each finding before the merge.
