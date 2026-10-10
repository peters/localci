mkdir -p bin docs
printf '#!/bin/sh\necho "value=42" >> "$GITHUB_OUTPUT"\n' > bin/cargo
chmod +x bin/cargo
echo '# Notes' > docs/notes.md
git add -A && git commit -qm base && git tag base
echo more >> docs/notes.md
