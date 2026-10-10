mkdir -p good bad
touch good/ok
echo keep > bad/.keep
git add -A && git commit -qm base
