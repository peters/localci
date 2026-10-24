# The folder that `localci setup` makes for Go, and no archive from the network.
mkdir -p toolchains/go/1.23.4/bin
printf '#!/bin/sh\necho "go version go1.23.4 linux/amd64"\n' > toolchains/go/1.23.4/bin/go
chmod +x toolchains/go/1.23.4/bin/go
