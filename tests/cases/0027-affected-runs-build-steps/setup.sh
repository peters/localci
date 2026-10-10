mkdir -p bin docs
printf '#!/bin/sh\nif [ "$1" = build ]; then mkdir -p out && touch out/app; fi\n' > bin/cargo
chmod +x bin/cargo
printf 'out/\n' > .gitignore
echo '# Notes' > docs/notes.md
git add -A && git commit -qm base && git tag base
echo more >> docs/notes.md
