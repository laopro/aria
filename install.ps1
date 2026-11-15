# Aria installer for Windows.
# Usage: irm https://raw.githubusercontent.com/laopro/aria/main/install.ps1 | iex
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"  # the progress bar makes downloads very slow

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

# Aria plays through yt-dlp; it looks for it next to aria.exe first.
$ytdlp = Join-Path $dest "yt-dlp.exe"
if (-not (Test-Path $ytdlp)) {
    Write-Host "Fetching yt-dlp (needed to play music)..."
    Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile $ytdlp
    Write-Host "Installed $ytdlp"
}

Write-Host "Done. Run: aria" -ForegroundColor Green
