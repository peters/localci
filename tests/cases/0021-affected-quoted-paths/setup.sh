mkdir -p bin src
printf '#!/bin/sh\nexit 0\n' > bin/cargo
chmod +x bin/cargo
echo 'pub fn f() {}' > 'src/café.rs'
git add -A && git commit -qm base && git tag base
echo 'pub fn g() {}' >> 'src/café.rs'
