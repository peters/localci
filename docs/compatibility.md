# Compatibility

This table shows how localci handles each workflow feature. Each row names
the test that keeps the behavior correct. Add a row for each new case folder.

| Feature | localci behavior | Test |
|---|---|---|
| `run:` steps | Run on the host with the shell of a GitHub-hosted runner. | case 0001 |
| `shell:` and `defaults.run` | `bash`, `sh`, `pwsh`, `powershell`, `python`, `cmd` and a custom `{0}` command. Workflow and job defaults apply. | case 0004 |
| Windows shells | `pwsh` by default, `cmd`, Git Bash for `bash`, and `python`. | case 0009 |
| `working-directory` | Relative to the checkout root. | case 0004 |
| `env` at each level | Workflow, job and step, with expressions. | case 0004 |
| `GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH` | Read after each step. Multi-line values with `<<` work. | cases 0001, 0004 |
| `steps.<id>.outputs` and `outcome` | Set after each step. | cases 0001, 0003 |
| Job `outputs` and `needs` | Jobs run in the order of `needs`. Outputs go to the next job. | case 0001 |
| `if:` with `success()`, `failure()`, `always()`, `cancelled()` | Same rules as GitHub. `cancelled()` is always false. | case 0003 |
| `continue-on-error` | The job stays green. The step shows `failure`. | case 0003 |
| `strategy.matrix` with `include` and `exclude` | Expanded with the rules of GitHub. Each combination is a leg. | unit tests in `workflow.rs` |
| `runs-on` from a matrix list | A list value gives the labels. | case 0002 |
| `runs-on` for another system | The leg does not run. The plan shows the labels that it needs. | case 0002 |
| YAML booleans and numbers as text | `run: true` and `name: 3` are text, as in GitHub. | case 0007 |
| `vars.LOCALCI` and `env.LOCALCI` | `true` in a local run, empty on GitHub. Use them in `if:` to skip a job or a step locally. See ADR-0008. | case 0008 |
| `services:` | Docker containers with health checks and mapped ports. | case 0006 |
| `RUNNER_TEMP` | A private folder (mode 700) for each job. localci warns when `TMPDIR` is writable by other users. | case 0010 |
| `timeout-minutes` | Stops the step process after the time. | not tested |
| `--affected` | Leaves out tool steps (Cargo, Node, dotnet, Go, Gradle, Swift) whose inputs did not change since the base. Unknown commands always run. See ADR-0009. | cases 0011 to 0013, 0016 to 0018, 0020 to 0022, 0024 to 0027 |
| `--cache` | Uses the result of a step that passed on the same inputs, with its outputs. See ADR-0010. | cases 0014, 0023 |
| `--parallel` | Runs independent legs at the same time, in the order of `needs`. A matrix job fails in `needs` when any leg failed. See ADR-0011. | cases 0015, 0019 |
| `uses:` actions | Not run. The step shows `skipped`. See ADR-0002. | case 0001 |
| Setup actions with `--toolchains` | `actions/setup-dotnet`, `actions/setup-node`, the Rust setup actions and `actions/setup-go` use the toolchain that `localci setup` installed, at the version that the workflow or its version file names. Python and Java are in the plan only. See ADR-0012. | cases 0028, 0029, 0030 |
| Reusable workflow calls | Not run. `localci check` shows a note. | none |
| `container:` | Ignored. The steps run on the host. | none |
| `concurrency`, `permissions` | Ignored. | none |
| Arithmetic in expressions | Not in the language. localci gives an error, the same as GitHub. | none |
