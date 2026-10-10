mkdir -p bin docs
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
echo '# Notes' > docs/notes.md
git add -A && git commit -qm base && git tag base
echo more >> docs/notes.md
