mkdir -p bin ci
printf '#!/bin/sh\nexit 0\n' > bin/cargo
printf '#!/bin/sh\necho "MODE=one" >> "$GITHUB_ENV"\n' > ci/setup.sh
chmod +x bin/cargo ci/setup.sh
git add -A && git commit -qm base && git tag base
printf '#!/bin/sh\necho "RUSTFLAGS=-Dwarnings" >> "$GITHUB_ENV"\n' > ci/setup.sh
