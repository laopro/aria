# Aria installer for Windows.
# Usage: irm https://raw.githubusercontent.com/laopro/aria/main/install.ps1 | iex
$ErrorActionPreference = "Stop"

$repo = "laopro/aria"
$dest = Join-Path $env:LOCALAPPDATA "Aria"
$exe = Join-Path $dest "aria.exe"

Write-Host "Fetching the latest Aria release..."
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$asset = $release.assets | Where-Object { $_.name -eq "aria-windows-x86_64.exe" } | Select-Object -First 1
if (-not $asset) { throw "could not find aria-windows-x86_64.exe in the latest release" }

New-Item -ItemType Directory -Force $dest | Out-Null
Invoke-WebRequest $asset.browser_download_url -OutFile $exe
Write-Host "Installed $exe"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$dest*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dest", "User")
    Write-Host "Added $dest to your PATH. Open a new terminal."
}

if (-not (Get-Command yt-dlp -ErrorAction SilentlyContinue)) {
    Write-Host "Heads up: install yt-dlp (https://github.com/yt-dlp/yt-dlp) so Aria can play music." -ForegroundColor Yellow
}

Write-Host "Done. Run: aria" -ForegroundColor Green
