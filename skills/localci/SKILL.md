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

Goal: open the PR only when localci is green and a local review is clean, and
let GitHub CI run on the first and the last head instead of on every fix.
If the repository's own instructions (AGENTS.md, CONTRIBUTING.md) set a
different PR process, follow those.

1. **localci green.** Run the subset that matches the diff (sections 2-5).
2. **Self-review, then the local review loop.** Always read the full diff
   yourself first. Then run Copilot CLI's built-in review with the prompt below.
   It uses the `gh` login and takes one to three minutes. Ask for concrete
   inputs: a general "review this" prompt misses most of what the review on
   GitHub finds later.

   ```sh
   base="$(git merge-base HEAD origin/HEAD 2>/dev/null || git merge-base HEAD origin/main)"
   copilot -p "/review the changes on this branch compared to commit $base (run git diff $base HEAD). Read each changed file in full, not only the changed lines, and the code it depends on. Find bugs: for each, give file:line, a one-line title, a concrete input or sequence of events, and the wrong result. Also check the pull request rules in AGENTS.md and CONTRIBUTING.md as a checklist: required evidence such as GIFs, tests and test procedures, files that must change together, and size limits. Only report problems with a concrete failure; skip style. If there are no significant issues, say so." \
     -s --reasoning-effort high --allow-tool='shell(git:*)' --deny-tool='write' --no-ask-user
   ```

   Fix the real findings, run localci for the affected jobs, commit, and review
   again until a round finds nothing actionable. Do not start a separate review
   agent: this loop and the review on GitHub are the independent reviews. If
   `copilot` is missing, offer `npm install -g @github/copilot`; your own
   review then is the local review.
3. **Open the PR** with a normal commit message. GitHub CI runs once. The
   review on GitHub is the final gate, not the place to iterate: request it
   only for a head that the local loop found clean.
4. **Review rounds.** When the review on GitHub reports findings, fix all of
   them, run the local review loop until it is clean again, run localci for
   the affected jobs, then push once with `[skip ci]` at the end of the commit
   message and request the next review. GitHub starts no workflow for a head
   commit that carries that marker.
5. **Final head.** When the reviewers are satisfied, push the head without the
   marker (amend the message or add an empty commit) so full CI runs on the
   exact head. Required checks stay "Expected" until that run passes. If the
   next GitHub round will probably be the last one, push the fix without the
   marker at once, so CI and the review run on the same head.
6. **Ready to merge** when CI passed on the exact head, no review thread is
   open, and the review on GitHub says "Approved", or says "Needs a closer
   look" with 0 open findings and no "Previously missed" findings while the
   local review on the same commit is clean. A vague summary without findings
   is not a finding. The repository's merge rules still apply, for example a
   person's explicit permission.
7. **Never on the default branch.** GitHub reads the whole commit message. Make
   sure no squash message or merge commit that lands on the default branch
   contains `[skip ci]`, or CI on that branch is skipped too.

## When localci gets something wrong

If localci does something different from GitHub, tell the user and offer to
open an issue at https://github.com/peters/localci with a minimal workflow that
shows the problem. Never copy content from a private workflow into an issue.
