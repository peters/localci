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
   network.
2. localci reads these actions and inputs:

   | Action | Inputs and files |
   |---|---|
   | `actions/setup-dotnet` | `dotnet-version` (one version for each line), `global-json-file`, else `global.json` in the checkout root |
   | `actions/setup-node` | `node-version`, `node-version-file` (`.nvmrc`, `.node-version`, `.tool-versions`, `package.json` with `volta.node` or `engines.node`) |
   | `dtolnay/rust-toolchain` | the `toolchain` input, else the reference after `@`; `components`, `targets` |
   | `actions-rust-lang/setup-rust-toolchain` | `toolchain`, else `rust-toolchain.toml` or `rust-toolchain`, else `stable` |
   | `actions/setup-go` | `go-version`, `go-version-file` (`go.mod`: the `toolchain` line wins over the `go` line) |
   | `actions/setup-python` | `python-version`, `python-version-file` |
   | `actions/setup-java` | `distribution`, `java-version`, `java-version-file` |

   An input can use the `matrix`, `github`, `vars`, `inputs` and `env`
   contexts, with the `env:` of the workflow, the job and the step. An input
   that an earlier step or job sets (through `steps.`, `needs.` or an `env:`
   value that reads them) is known only during the run. The plan says so. It
   does not use a version file in that case, because the action does not
   either.
3. localci does not guess. A range such as `>=18` or `^20` stays a request.
   For `global.json`, the plan shows the `rollForward` value as the policy and
   uses the version in the file.
4. `localci setup` installs the .NET SDK, Node.js and Rust. Go, Python and
   Java are in the plan only, and the setup says that it did not install them.
5. Each download comes from the official source, over HTTPS, and localci
   checks it before it uses it:

   | Tool | Source | Check |
   |---|---|---|
   | .NET SDK | the .NET release metadata (`releases-index.json`, then `releases.json` of the channel) | SHA-512 from `releases.json` |
   | Node.js | `https://nodejs.org/dist` and its `index.json` | SHA-256 from `SHASUMS256.txt` |
   | Rust | `rustup-init` from `https://static.rust-lang.org/rustup/dist` | SHA-256 from the `.sha256` file; rustup then checks each toolchain itself |

   A wrong checksum stops the install. localci deletes the download.
6. The toolchains folder is `--toolchains DIR`, else `LOCALCI_TOOLCHAINS`, else
   `$XDG_CACHE_HOME/localci/toolchains`, else `~/.cache/localci/toolchains`.
   Each Node.js version has its own folder, `node/<version>/`. The .NET SDKs
   share one root, `dotnet/`, as the official install script and
   `actions/setup-dotnet` make it: each SDK adds its versioned folders, and only
   a newer SDK replaces the top-level `dotnet` host. So a step that asks for
   `8.0.x` and `10.0.x` gets both SDKs and both runtimes. `dotnet/.versions/`
   marks each complete SDK. Rust uses one rustup home, `rust/`. localci unpacks into a temporary folder and then renames it,
   so a stopped setup does not leave a half folder. A lock file, `.lock`, stops
   a second setup until the first one ends. A lock that is older than one hour
   is stale. localci never uses `sudo`.
7. For a request that is not exact, for example `20` or `8.0.x`, localci
   writes the version that it installed to `requests.json`. A run then finds it
   without the network.
8. With `localci run --toolchains DIR` (or `LOCALCI_TOOLCHAINS`), a setup step
   does not show `skipped`. It puts the toolchain of `DIR` first on `PATH` and
   sets the variables of the tool: `DOTNET_ROOT` for .NET, and `RUSTUP_HOME`
   and `RUSTUP_TOOLCHAIN` for Rust. `RUSTUP_TOOLCHAIN` is not set when the
   checkout has a toolchain file, because that file wins on GitHub too.
   `CARGO_HOME` stays the user's, so the registry cache stays writable. When
   a toolchain is not in `DIR`, the step shows `skipped` with the reason, for
   example `Node.js 22.0.0 is not in DIR (run localci setup)`.
9. A run only reads `DIR`. A user who cannot write to `DIR` can use it.

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

## Checks

- Case 0028 shows the plan of each known action, the `env` inputs, the version
  files and an input that a job sets during the run.
- Case 0029 shows a run that uses a toolchain of the folder, a setup step
  without a version and a missing toolchain.
- Unit tests in `install.rs` refuse a wrong checksum. Unit tests in
  `install_sources.rs` pick the .NET SDK and the Node.js version from metadata.
