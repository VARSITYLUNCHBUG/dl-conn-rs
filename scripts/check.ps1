# Everything that must pass before calling a change done, without admin rights. A running dl-rs
# locks target\release\dl-rs.exe, so in that case the release build goes to target\next instead.
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
$bad = Get-ChildItem src, examples -Filter *.rs -Recurse | Select-String -Pattern '[^\x00-\x7F]' -CaseSensitive
if ($bad) {
    $bad | Select-Object -First 10 | ForEach-Object { "$($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
    $failed += 'ascii'
}

Write-Output '--- commit rule'
if ((git config core.hooksPath) -ne '.githooks') {
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
    $locked = @(Get-Process dl-rs -ErrorAction SilentlyContinue | Where-Object {
        try { $_.Path -like "$root\target\release\*" } catch { $true }
    }).Count -gt 0
    $target = if ($locked) { 'target\next' } else { 'target' }
    if ($locked) { Write-Output 'dl-rs is running from target\release, so building to target\next' }
    Step 'release build' { cargo build --release --quiet --target-dir $target }
    $exe = Join-Path $target 'release\dl-rs.exe'
    if (Test-Path $exe) { Write-Output ("{0}: {1:N0} KB" -f $exe, ((Get-Item $exe).Length / 1000)) }
}

if ($failed.Count) {
    Write-Output "FAILED: $($failed -join ', ')"
    exit 1
}
Write-Output 'all checks passed'
