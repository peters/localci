printf '{ "sdk": { "version": "8.0.404", "rollForward": "latestFeature" } }\n' > global.json
printf '{ "name": "x", "engines": { "node": ">=20" } }\n' > package.json
printf 'module example\n\ngo 1.22\ntoolchain go1.22.5\n' > go.mod
printf '3.12\n' > .python-version
printf '[toolchain]\nchannel = "1.81.0"\ncomponents = ["rustfmt"]\ntargets = ["wasm32-unknown-unknown"]\n' > rust-toolchain.toml
