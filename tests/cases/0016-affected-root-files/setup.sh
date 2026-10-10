mkdir -p bin crates/app/src
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
printf '[workspace]\nmembers = ["crates/app"]\n' > Cargo.toml
echo 'pub fn instructions() -> &'"'"'static str { "NOTES.md" }' > crates/app/src/lib.rs
echo '# Notes' > NOTES.md
git add -A && git commit -qm base && git tag base
echo more >> NOTES.md
