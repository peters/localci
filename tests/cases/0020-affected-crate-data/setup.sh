mkdir -p bin crates/app/src crates/app/tests/data docs
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
printf '[workspace]\nmembers = ["crates/app"]\n' > Cargo.toml
printf '[package]\nname = "app"\nversion = "0.1.0"\n' > crates/app/Cargo.toml
echo 'pub fn f() {}' > crates/app/src/lib.rs
echo '{}' > crates/app/tests/data/case.json
echo '# Notes' > docs/notes.md
git add -A && git commit -qm base && git tag base
echo '{"x": 1}' > crates/app/tests/data/case.json
