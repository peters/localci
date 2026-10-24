# A checkout with version files and no setup step, as most repositories have it.
printf '20.11.1\n' > .nvmrc
printf 'module example\n\ngo 1.23.4\n' > go.mod
