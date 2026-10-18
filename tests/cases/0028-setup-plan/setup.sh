printf '{ "sdk": { "version": "8.0.404", "rollForward": "latestFeature" } }\n' > global.json
printf '{ "name": "x", "engines": { "node": ">=20" } }\n' > package.json
printf 'module example\n\ngo 1.22\ntoolchain go1.22.5\n' > go.mod
printf '3.12\n' > .python-version
printf '[toolchain]\nchannel = "1.81.0" # pinned\ncomponents = [\n  "rustfmt",\n  '"'"'clippy'"'"',\n]\ntargets = ["wasm32-unknown-unknown"]\n' > rust-toolchain.toml
printf '{\n  // pinned for the build\n  "sdk": { "version": "9.0.100" }\n}\n' > sdk.json
