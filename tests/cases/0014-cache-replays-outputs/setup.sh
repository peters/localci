mkdir -p bin scripts src web docs
printf '#!/bin/sh\necho "value=42" >> "$GITHUB_OUTPUT"\n' > bin/cargo
printf '#!/bin/sh\nexit 0\n' > bin/npm
chmod +x bin/cargo bin/npm
echo '# Notes' > docs/notes.md
git add -A && git commit -qm base
