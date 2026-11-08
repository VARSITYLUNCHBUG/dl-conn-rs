# Builds the release files to give to other people: the exe with build paths removed, its MD5 and
# SHA-256 hashes, the licence and crate notices, and a source zip of the committed tree. The exe is
# signed if a certificate is set up. Nothing is uploaded.
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 [-OutDir dist]
# To sign, set DL_CONN_RS_SIGN_THUMBPRINT to a certificate's thumbprint in your Windows certificate store.
# Never put a key or password in this repo.
param(
    [string]$OutDir = 'dist',
    [switch]$BuildOnly
)
$ErrorActionPreference = 'Stop'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
# Setting this replaces the flags in .cargo\config.toml, so the static CRT is repeated here. The
# longest prefixes come first, because the compiler applies the last match.
$env:RUSTFLAGS = @(
    '-C target-feature=+crt-static',
    '-C control-flow-guard',
    "--remap-path-prefix=$env:USERPROFILE=user",
    "--remap-path-prefix=$rustupHome=rustup",
    "--remap-path-prefix=$cargoHome\registry\src=crates",
    "--remap-path-prefix=$root=dl-conn-rs"
) -join ' '
cargo build --release --locked --target-dir target\dist
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$exe = Join-Path $root 'target\dist\release\dl-conn-rs.exe'

# Anything that identifies this PC or its user mustn't be in something given to others.
$text = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($exe))
$leaks = @($env:USERNAME, $env:USERPROFILE, $root, '\Users\', '\.cargo\', '\.rustup\') | Where-Object { $_ -and $text.Contains($_) }
if ($leaks) {
    Write-Output "The exe contains machine paths: $($leaks -join ', ')"
    exit 1
}
Write-Output ("{0}: {1:N1} MB, no machine paths" -f $exe, ((Get-Item $exe).Length / 1MB))
if ($BuildOnly) { exit 0 }

New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = Join-Path $OutDir 'dl-conn-rs.exe'
Copy-Item $exe $out -Force

# Every crate in the exe and its licence, as their licences require.
$metadata = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
$node = $metadata.resolve.nodes
$normal = @{}
$pending = New-Object System.Collections.Queue
$pending.Enqueue($metadata.resolve.root)
while ($pending.Count) {
    $id = $pending.Dequeue()
    if ($normal[$id]) { continue }
    $normal[$id] = $true
    foreach ($dependency in ($node | Where-Object { $_.id -eq $id }).deps) {
        if ($dependency.dep_kinds | Where-Object { -not $_.kind }) { $pending.Enqueue($dependency.pkg) }
    }
}
$notices = @('dl-conn-rs uses these open source crates, under these licences:', '')
$metadata.packages | Where-Object { $normal[$_.id] -and $_.name -ne 'dl-conn-rs' } | Sort-Object name, version | ForEach-Object {
    $notices += "$($_.name) $($_.version): $($_.license)"
}
Set-Content (Join-Path $OutDir 'THIRD_PARTY_NOTICES.txt') $notices -Encoding ASCII

if ($env:DL_CONN_RS_SIGN_THUMBPRINT) {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" | Sort-Object FullName | Select-Object -Last 1
    if (-not $signtool) { throw 'signtool.exe not found. Install the Windows SDK.' }
    & $signtool.FullName sign /sha1 $env:DL_CONN_RS_SIGN_THUMBPRINT /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $out
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'Signed.'
} else {
    Write-Output 'Not signed: DL_CONN_RS_SIGN_THUMBPRINT is not set, so users will see "Unknown publisher".'
}
foreach ($algorithm in 'MD5', 'SHA256') {
    $hash = (Get-FileHash $out -Algorithm $algorithm).Hash.ToLower()
    Set-Content (Join-Path $OutDir "dl-conn-rs.exe.$($algorithm.ToLower())") "$hash  dl-conn-rs.exe" -Encoding ASCII
    Write-Output "${algorithm}: $hash"
}

Copy-Item (Join-Path $root 'LICENSE') (Join-Path $OutDir 'LICENSE.txt') -Force
$version = ($metadata.packages | Where-Object { $_.name -eq 'dl-conn-rs' }).version
if (git status --porcelain) { Write-Output 'Note: uncommitted changes are not in the source zip.' }
$source = Join-Path $OutDir "dl-conn-rs-$version-source.zip"
git archive --format=zip --prefix="dl-conn-rs-$version/" -o $source HEAD
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "Source: $source"
