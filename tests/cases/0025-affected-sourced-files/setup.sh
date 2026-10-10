mkdir -p bin ci
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
printf 'python3 -c "print(1)"\n. ci/vars.sh\n' > ci/setup.sh
printf 'echo "MODE=one" >> "$GITHUB_ENV"\n' > ci/vars.sh
git add -A && git commit -qm base && git tag base
printf 'echo "RUSTFLAGS=-Dwarnings" >> "$GITHUB_ENV"\n' > ci/vars.sh
