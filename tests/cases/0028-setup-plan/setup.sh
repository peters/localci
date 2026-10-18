printf '{ "sdk": { "version": "8.0.404", "rollForward": "latestFeature" } }\n' > global.json
printf '{ "name": "x", "engines": { "node": ">=20" } }\n' > package.json
printf 'module example\n\ngo 1.22\ntoolchain go1.22.5\n' > go.mod
printf '3.12\n' > .python-version
printf '[toolchain]\nchannel = "1.81.0" # pinned\nprofile = "default"\ncomponents = [\n  "rustfmt",\n  '"'"'clippy'"'"',\n]\ntargets = ["wasm32-unknown-unknown"]\n' > rust-toolchain.toml
printf '\357\273\277{\n  // pinned for the build\n  "sdk": { "version": "9.0.100" }\n}\n' > sdk.json
printf '[project]\nname = "x"\nrequires-python = ">=3.11"\n' > pyproject.toml
printf 'nodejs 20.1.0\npython 3.12.4\n' > .tool-versions
ln -s /etc/hosts linked-version
