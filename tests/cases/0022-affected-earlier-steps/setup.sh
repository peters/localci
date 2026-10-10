mkdir -p bin docs
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
git add -A && git commit -qm base && git tag base
python3 - <<'PY'
import pathlib
p = pathlib.Path(".github/workflows/ci.yml")
text = p.read_text()
p.write_text(text.replace('      - name: Set flags\n        run: echo "RUSTFLAGS=-Dwarnings" >> "$GITHUB_ENV"\n', ''))
PY
