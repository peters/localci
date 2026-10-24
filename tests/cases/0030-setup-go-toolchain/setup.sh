# The folder that `localci setup` makes for Go, and the version that it remembered
# for the request of go.mod. CI does not download an archive.
mkdir -p ../0030-toolchains/go/1.22.5/bin
printf '#!/bin/sh\necho "go version go1.22.5 linux/amd64"\n' > ../0030-toolchains/go/1.22.5/bin/go
chmod +x ../0030-toolchains/go/1.22.5/bin/go
printf '{ "go 1.22": "1.22.5" }\n' > ../0030-toolchains/requests.json
printf 'module example\n\ngo 1.22\ntoolchain go1.22.5\n' > go.mod
