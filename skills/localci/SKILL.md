---
name: localci
description: Run the GitHub Actions jobs that match this computer's OS locally with the localci binary, to get CI feedback before a push when GitHub is slow or offline. Use before pushing or opening a PR, after fixing review findings, or when the user asks to run CI locally. The agent proposes which jobs and steps to run from the diff and lets the user choose.
---

# localci

`localci` reads `.github/workflows/*.yml` and runs the `run:` steps of the jobs
whose `runs-on` matches this OS, in the current checkout. `uses:` actions are
skipped. Local results are a fast pre-check; GitHub CI stays the final gate.

## 1. Make sure the binary exists

```sh
command -v localci || curl -fsSL https://raw.githubusercontent.com/peters/localci/main/install.sh | sh
```

On Windows: `irm https://raw.githubusercontent.com/peters/localci/main/install.ps1 | iex`.

## 2. Look before running

Run these in the repository root. Neither runs any workflow commands.

```sh
localci check                      # every workflow parses; every expression evaluates
localci plan --json                # legs, steps, step kinds, and why a step would not run
git diff --stat "$(git merge-base HEAD origin/HEAD 2>/dev/null || git merge-base HEAD origin/main)"
```

`-w NAME` selects another workflow file. The default is `ci.yml`. Never run
release, deploy or publish workflows unless the user asks for that file by name.

## 3. Propose a subset and let the user choose

From the plan and the diff, propose the smallest set that catches what CI
would catch for this change:

- Keep cheap checks that fail often: format, lint, repository checks.
- Keep test legs whose code the diff touches. Drop legs for areas the diff does
  not touch, and say why.
- Steps with `"kind": "setup"` install tools (apt-get, brew, installers). If the
  tools are already on this machine (check with `command -v`), propose
  `--skip-setup`. Never run a `sudo` step without asking.
- Legs with `"runnable": false` need another OS. List them as not covered.
- `uses:` setup actions (setup-dotnet, setup-node, the Rust actions) are skipped
  unless the machine has the tools. On a fresh machine, run `localci setup` first
  (`localci setup --plan` shows the versions), then `localci run --toolchains DIR`.

Ask one question with AskUserQuestion (multiSelect) listing the runnable legs,
with your recommendation first and marked "(Recommended)". Skip the question if
the user already said what to run, or said to just run it.

## 4. Run

While you iterate, use `--fast`. It leaves out check steps (tests, lint,
format) whose inputs did not change since the merge base, reuses steps that
passed on the same inputs, and runs independent jobs at the same time.
`localci plan --affected` shows each decision and its reason first. Steps that
localci cannot read (for example Python scripts) always run, so for a change
that touches one area, naming the job is faster still.

```sh
localci run --fast -q --json                      # quick feedback while you iterate
localci run repo-checks -q --json                 # only the job that the diff touches
localci run <leg keys or job ids> [--skip TEXT | --skip LEG:N]... [--only TEXT] [--skip-setup] -q --json
```

Before the PR opens, run once without `--fast`, so every picked step really runs
on the exact commit.

- `-q` hides step output and prints the tail of a failing step's log.
- `--json` prints the report on stdout. The report is also saved as
  `report.json` in the logs folder printed at the end.
- Use `run_in_background` for runs longer than a few minutes, and wait for the
  completion notice instead of polling.
- Add `-k` to continue after a failing leg when the user wants every failure at once.
- If a service fails because a port is in use, the local process already runs
  that service. Propose `--no-services`.
- Do not use `--ignore-if` unless the user asks. It runs jobs GitHub would skip.

## Permanent local skips

If the same step must never run locally (a `sudo` installer, a deploy job),
propose a condition in the workflow instead of a flag on every run. localci
sets `LOCALCI`; on GitHub it is empty, so GitHub still runs the step:

- Job: `if: ${{ !vars.LOCALCI }}`
- Step: `if: ${{ !env.LOCALCI }}`

## 5. On failure

1. Read only the failing step's log: `legs[].steps[].log` for the step at `failed_step`.
2. Fix the cause.
3. Re-run that step alone with `--only LEG:N`, then the whole leg.

## 6. Report

Give a short table of legs with pass, fail or skipped, the time, and the legs
that need another OS. Say clearly that this is a local run and not a GitHub check.

## Before and during a pull request

1. Run the localci subset that matches the diff (sections 2-5).
2. The implementing agent self-reviews the complete diff once, fixes its findings,
   and runs the affected checks before the first push. Do not run a separate
   review agent or a CLI review loop.
3. Open the PR with a normal commit message. The upstream repository starts
   PR review automatically. Never assign, request or re-request an automated
   reviewer through labels, comments, commands, APIs, workflow dispatches or
   PR state changes.
4. Read PR feedback, fix valid findings and answer invalid findings with evidence.
   Run localci for affected jobs and push the fixes without another full
   self-review round or a request for another review.
5. Before merge, require green CI on the exact head, the required upstream
   review decision, and the repository's feedback and acceptance gates. Check
   which commit the review covers. Report a missing or failed required review;
   do not trigger one yourself. The repository's merge authorization still applies.
6. Never put a CI-skip marker in a commit that lands on the default branch.

## When localci gets something wrong

If localci does something different from GitHub, tell the user and offer to
open an issue at https://github.com/peters/localci with a minimal workflow that
shows the problem. Never copy content from a private workflow into an issue.
