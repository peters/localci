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
