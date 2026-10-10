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

Ask one question with AskUserQuestion (multiSelect) listing the runnable legs,
with your recommendation first and marked "(Recommended)". Skip the question if
the user already said what to run, or said to just run it.

## 4. Run

```sh
localci run <leg keys or job ids> [--skip TEXT | --skip LEG:N]... [--only TEXT] [--skip-setup] -q --json
```

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

## When localci gets something wrong

If localci does something different from GitHub, tell the user and offer to
open an issue at https://github.com/peters/localci with a minimal workflow that
shows the problem. Never copy content from a private workflow into an issue.
