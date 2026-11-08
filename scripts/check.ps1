# Everything that must pass before calling a change done, without admin rights. The release build
# goes to target\dist, so a running dl-conn-rs elsewhere can't lock it, and it fails on any machine path.
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root
$failed = @()

function Step($name, [scriptblock]$action) {
    Write-Output "--- $name"
    & $action
    if ($LASTEXITCODE -ne 0) { $script:failed += $name }
}

Step 'format' { cargo fmt --check }

Write-Output '--- ascii'
$files = @(Get-ChildItem src, examples -Filter *.rs -Recurse) + @(Get-ChildItem README.md, docs -Filter *.md -Recurse)
$bad = $files | Select-String -Pattern '[^\x00-\x7F]' -CaseSensitive
if ($bad) {
    $bad | Select-Object -First 10 | ForEach-Object { "$($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
    $failed += 'ascii'
}

Write-Output '--- commit rule'
# Worktrees may point at the main checkout's folder by absolute path.
if ((git config core.hooksPath) -notmatch '(^|[\\/])\.githooks[\\/]?$') {
    Write-Output 'core.hooksPath is not set. Run: git config core.hooksPath .githooks'
    $failed += 'commit rule'
}
$patterns = Get-Content .githooks\attribution-patterns.txt -Encoding UTF8 | Where-Object { $_ }
foreach ($commit in git log --all --format='%h') {
    $text = (git log -1 --format='%an %ae%n%cn %ce%n%B' $commit) -join "`n"
    $hits = @($patterns | Where-Object { $text -match $_ })
    if ((git log -1 --format='%an %ae %cn %ce' $commit) -match 'claude|anthropic') { $hits += 'identity' }
    if ($hits.Count) {
        Write-Output "$commit carries a Claude signature: $($hits -join ', ')"
        $failed += 'commit rule'
    }
}

Step 'clippy' { cargo clippy --all-targets --quiet -- -D warnings }
Step 'unit tests' { cargo test --quiet }

if (-not $SkipBuild) {
    Step 'release build' { & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'scripts\release.ps1') -BuildOnly }
}

if ($failed.Count) {
    Write-Output "FAILED: $($failed -join ', ')"
    exit 1
}
Write-Output 'all checks passed'
