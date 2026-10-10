mkdir -p bin scripts src web docs
printf '#!/bin/sh\necho "value=42" >> "$GITHUB_OUTPUT"\n' > bin/cargo
printf '#!/bin/sh\nexit 0\n' > bin/npm
chmod +x bin/cargo bin/npm
git add -A && git commit -qm base && git tag base
python3 - <<'PY'
import pathlib
p = pathlib.Path(".github/workflows/ci.yml")
p.write_text(p.read_text().replace("cargo clippy", "cargo clippy -- -D warnings"))
PY
