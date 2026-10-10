mkdir -p bin scripts src web docs
printf '#!/bin/sh\necho "value=42" >> "$GITHUB_OUTPUT"\n' > bin/cargo
printf '#!/bin/sh\nexit 0\n' > bin/npm
chmod +x bin/cargo bin/npm
cat > scripts/shard.sh <<'SCRIPT'
run_cargo() {
  cargo "$@"
}
case "$1" in
  *) run_cargo test ;;
esac
SCRIPT
echo 'pub fn f() {}' > src/lib.rs
echo x > web/app.js
echo '# Guide' > docs/guide.md
git add -A && git commit -qm base && git tag base
echo more >> docs/guide.md
echo y >> web/app.js
