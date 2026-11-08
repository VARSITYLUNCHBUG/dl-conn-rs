# Publishes docs\wiki to the GitHub wiki. The wiki repo exists only after its first page is saved
# on the website, so create any page there once before the first run.
#   powershell -ExecutionPolicy Bypass -File scripts\publish-wiki.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$url = 'https://github.com/VARSITYLUNCHBUG/dl-conn-rs.wiki.git'
$work = Join-Path ([IO.Path]::GetTempPath()) "dl-conn-rs-wiki-$PID"

git clone --quiet $url $work
if ($LASTEXITCODE -ne 0) { throw "Couldn't clone $url. Save one page on the wiki website first." }
try {
    Get-ChildItem $work -Filter *.md | Remove-Item
    Copy-Item (Join-Path $root 'docs\wiki\*.md') $work
    git -C $work add -A
    if (-not (git -C $work status --porcelain)) {
        Write-Output 'The wiki is already up to date.'
        exit 0
    }
    $name = git -C $root config user.name
    $email = git -C $root config user.email
    if (-not $name) { $name = 'VARSITYLUNCHBUG'; $email = '100867870+VARSITYLUNCHBUG@users.noreply.github.com' }
    git -C $work -c user.name=$name -c user.email=$email commit --quiet -m 'Update wiki from docs/wiki'
    git -C $work push --quiet origin HEAD
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'Wiki published: https://github.com/VARSITYLUNCHBUG/dl-conn-rs/wiki'
} finally {
    Set-Location $root
    Remove-Item -Recurse -Force $work
}
