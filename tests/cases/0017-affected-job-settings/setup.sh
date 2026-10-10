mkdir -p bin docs
printf '#!/bin/sh\necho "value=42" >> "$GITHUB_OUTPUT"\n' > bin/cargo
chmod +x bin/cargo
git add -A && git commit -qm base && git tag base
python3 - <<'PY'
import pathlib
p = pathlib.Path(".github/workflows/ci.yml")
p.write_text(p.read_text().replace("MODE: one", "MODE: two"))
PY
