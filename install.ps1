# Install localci from the latest GitHub release.
#   irm https://raw.githubusercontent.com/peters/localci/main/install.ps1 | iex
# $env:LOCALCI_INSTALL_DIR picks the folder. $env:LOCALCI_VERSION picks a tag.
$ErrorActionPreference = 'Stop'
$repo = 'peters/localci'
$dir = if ($env:LOCALCI_INSTALL_DIR) { $env:LOCALCI_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\localci' }
$version = if ($env:LOCALCI_VERSION) { $env:LOCALCI_VERSION } else { 'latest' }
$asset = 'localci-x86_64-pc-windows-msvc.zip'
if ($version -eq 'latest') {
    # The newest release, pre-releases included; GitHub's "latest" link skips those.
    $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=1")[0].tag_name
}
$base = "https://github.com/$repo/releases/download/$version"

$work = Join-Path ([IO.Path]::GetTempPath()) ("localci-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host "localci: downloading $asset"
    Invoke-WebRequest "$base/$asset" -OutFile (Join-Path $work $asset)
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS')
    $line = Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object { $_ -match " $([regex]::Escape($asset))$" }
    $expected = ($line -split ' ')[0]
    $actual = (Get-FileHash (Join-Path $work $asset) -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) { throw "checksum mismatch for $asset" }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Expand-Archive (Join-Path $work $asset) -DestinationPath $dir -Force
} finally {
    Remove-Item -Recurse -Force $work
}
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not ($userPath -split ';' -contains $dir)) {
    [Environment]::SetEnvironmentVariable('Path', "$dir;$userPath", 'User')
    Write-Host "localci: added $dir to your user PATH; open a new terminal"
}
Write-Host "localci: installed $(& (Join-Path $dir 'localci.exe') --version)"
Write-Host "localci: next, run 'localci skill install' to teach your coding agent"
