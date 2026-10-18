# ADR-0012: Install the toolchains that setup actions declare

Status: Accepted.
Date: 2026-10-18.

This record adds to ADR-0002. ADR-0002 stays correct when you do not use
`--toolchains`.

## Context

ADR-0002 leaves out `uses:` steps, because a developer computer has the tools.
A new machine does not have them, for example a cloud machine of an agent.
The workflow already declares the tools: `actions/setup-dotnet`,
`actions/setup-node` and the Rust setup actions name a version or a file with
the version. When localci reads these declarations, CI stays the only list of
tools. A second list can become different from CI.

## Decision

1. `localci setup --plan` shows, for the picked legs, each toolchain that a
   setup step declares. It shows the version, the source of the version and
   one of these states: `installs`, `plan only`, `nothing to install` or
   `not supported`. `--json` gives the same data as JSON. The plan needs no
   network. A setup step that `--skip` names, or whose `if:` is false before
   the run, is not in the plan, and `localci setup` does not install it.
2. localci reads these actions and inputs:

   | Action | Inputs and files |
   |---|---|
   | `actions/setup-dotnet` | each line of `dotnet-version`, and the SDK of `global-json-file`; without both, `global.json` in the checkout root. `global.json` can have comments |
   | `actions/setup-node` | `node-version`, else `node-version-file` (`.nvmrc`, `.node-version`, `.tool-versions`, `package.json` with `volta.node` or `engines.node`) |
   | `dtolnay/rust-toolchain` | the `toolchain` input, else the reference after `@`; `components`, `targets` |
   | `actions-rust-lang/setup-rust-toolchain` | `toolchain`, else `rust-toolchain` or `rust-toolchain.toml` (in this order, as rustup reads them), else `stable`. The action also sets `RUSTFLAGS` (input `rustflags`, default `-D warnings`) and `CARGO_INCREMENTAL=0` |
   | `actions/setup-go` | `go-version`, else `go-version-file` (`go.mod`: the `toolchain` line wins over the `go` line) |
   | `actions/setup-python` | `python-version`, else `python-version-file` |
   | `actions/setup-java` | `distribution`, and `java-version` or `java-version-file` |

   An input can use the `matrix`, `github`, `vars`, `inputs` and `env`
   contexts, with the `env:` of the workflow, the job and the step. An input
   that an earlier step or job sets (through `steps.`, `needs.` or an `env:`
   value that reads them) is known only during the run. The plan says so. It
   does not use a version file in that case, because the action does not
   either. Only the inputs that name a version or its file decide this.
3. A version file must be in the checkout. localci does not read a path
   outside it (absolute, with `..`, or a symlink that leads out), so a plan
   never shows a file of the machine. A version or toolchain name must be a plain name (letters, digits,
   `.`, `_`, `+` and `-`, with a letter or digit first), so it cannot point
   outside the toolchains folder or become an option of a command.
4. localci does not guess. A range such as `>=18` or `^20` stays a request.
   For `global.json`, the plan shows the `rollForward` value as the policy and
   uses the version in the file.
5. `localci setup` installs the .NET SDK, Node.js and Rust. Go, Python and
   Java are in the plan only, and the setup says that it did not install them.
6. Each download comes from the official source, over HTTPS, and localci
   checks it before it uses it:

   | Tool | Source | Check |
   |---|---|---|
   | .NET SDK | the .NET release metadata (`releases-index.json`, then `releases.json` of the channel) | SHA-512 from `releases.json` |
   | Node.js | `https://nodejs.org/dist` and its `index.json` | SHA-256 from `SHASUMS256.txt` |
   | Rust | `rustup-init` from `https://static.rust-lang.org/rustup/dist` | SHA-256 from the `.sha256` file; rustup then checks each toolchain itself |

   localci checks the bytes of the download in memory and unpacks the same
   bytes, so nothing can change the file between the check and the use. A
   wrong checksum stops the install.
7. The toolchains folder is `--toolchains DIR`, else `LOCALCI_TOOLCHAINS`, else
   `$XDG_CACHE_HOME/localci/toolchains`, else `~/.cache/localci/toolchains`.
   localci makes it an absolute path.
   - Each Node.js version has its own folder, `node/<version>/`.
   - The .NET SDKs share one root, `dotnet/`, as the official install script
     and `actions/setup-dotnet` make it: each SDK adds its versioned folders,
     and only a newer SDK replaces the top-level `dotnet` host. So a step that
     asks for `8.0.x` and `10.0.x` gets both SDKs and both runtimes.
     `dotnet/.versions/` marks each complete SDK.
   - Rust uses one rustup home, `rust/`. A toolchain counts as installed when
     its channel manifest is there and it has each requested component and
     target. localci installs the minimal profile. A toolchain file with the
     `default` or `complete` profile adds `clippy` and `rustfmt`. A channel
     such as `stable` is updated at each setup, as on GitHub; when rustup
     cannot reach its source, the installed toolchain stays.

   localci unpacks into a new temporary folder and then renames it, so a
   stopped setup does not leave a half folder. An operating system lock on
   `.lock` stops a second setup until the first one ends. The lock ends with
   the process, also after Ctrl-C or a kill. The next setup removes the
   temporary folders that a kill left. localci never uses `sudo`.
8. For a request that is not exact, for example `20` or `8.0.x`, localci
   writes the version that it installed to `requests.json`. A run then finds it
   without the network. A `requests.json` that localci cannot read stops the
   setup, and localci does not replace it. An exact version that is installed
   needs no network. When the source of a request that is not exact cannot be
   reached, `localci setup` uses the version that it remembered.
9. With `localci run --toolchains DIR` (or `LOCALCI_TOOLCHAINS`), a setup step
   does not show `skipped`. It puts the toolchain of `DIR` first on `PATH` and
   sets the variables of the tool and the action: `DOTNET_ROOT` for .NET,
   `RUSTUP_HOME` and `RUSTUP_TOOLCHAIN` for Rust, and the variables of item 2.
   It also adds the tool and the installed version to `LOCALCI_TOOLCHAINS`,
   for example `rust 1.83.0 dotnet 8.0.426`. This variable is part of the key
   of `--cache` (ADR-0010), so a new version of a channel such as `stable`
   does not reuse a result of the old version.
   `RUSTUP_TOOLCHAIN` is not set when the checkout root has a toolchain file,
   because that file wins on GitHub too. `CARGO_HOME` stays the user's, so the
   registry cache stays writable. When a toolchain is not in `DIR`, the step
   shows `skipped` with the reason, for example
   `Node.js 22.0.0 is not in DIR (run localci setup)`. The setup step follows
   its `if:` and `--skip`. With `--only`, it stays active, because the later
   steps need its toolchain.
10. A run only reads `DIR`. A user who cannot write to `DIR` can use it.

## Consequences

- An agent on a new machine runs `localci setup` and then `localci run` with
  the tools that CI uses, at the versions that CI uses.
- The first setup downloads several hundred megabytes for each .NET SDK and
  each Rust toolchain.
- A setup action that localci does not know shows `not supported`. Install
  that tool on the host, as before.
- A version that an earlier step or job sets is known only during the run.
  `localci setup` cannot install it in advance.
- localci trusts the HTTPS connection to the official source for the
  checksum. It does not check a signature.
- All .NET SDKs of the folder are in one root, as on a GitHub runner, where
  setup-dotnet adds SDKs next to the SDKs of the runner image. Without
  `global.json`, the newest SDK of the root is the default, on GitHub too.
- `RUSTUP_TOOLCHAIN` is not set when the checkout root has a toolchain file. A
  toolchain file in a subfolder does not change that, so there the step's
  toolchain wins, unlike on GitHub.

## Checks

- Case 0028 shows the plan of each known action, the `env` inputs, the version
  files and an input that a job sets during the run.
- Case 0029 shows a run that uses a toolchain of the folder, a setup step
  without a version and a missing toolchain.
- Unit tests in `install.rs` refuse a wrong checksum. Unit tests in
  `install_sources.rs` pick the .NET SDK and the Node.js version from metadata.
