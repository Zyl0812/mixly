#Requires -Version 5.1
# Install mixly: irm https://raw.githubusercontent.com/Zyl0812/mixly/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'

$Repo  = 'Zyl0812/mixly'
$Asset = 'mixly-x86_64-pc-windows-msvc.zip'
$Dest  = Join-Path $env:LOCALAPPDATA 'Programs\mixly'

Write-Host "==> downloading $Asset"
$tmp = Join-Path ([IO.Path]::GetTempPath()) "mixly-$([guid]::NewGuid()).zip"
# ponytail: no checksum, GitHub over TLS is the trust anchor; add one if releases get mirrored
Invoke-WebRequest "https://github.com/$Repo/releases/latest/download/$Asset" -OutFile $tmp

New-Item -ItemType Directory -Force $Dest | Out-Null
Expand-Archive $tmp -DestinationPath $Dest -Force
Remove-Item $tmp

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$Dest*") {
    $new = if ($userPath) { "$userPath;$Dest" } else { $Dest }
    [Environment]::SetEnvironmentVariable('Path', $new, 'User')
    Write-Host "==> added $Dest to user PATH (open a new shell to pick it up)"
}
$env:Path = "$env:Path;$Dest"

if (-not (Get-Command mpv -ErrorAction SilentlyContinue)) {
    Write-Host '==> mpv not found, installing'
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        winget install --id shinchiro.mpv -e --accept-package-agreements --accept-source-agreements
    } elseif (Get-Command scoop -ErrorAction SilentlyContinue) {
        scoop install mpv
    } else {
        Write-Warning 'no winget/scoop found. install mpv from https://mpv.io/installation/, then set player.mpv_path in config.toml'
    }
}

Write-Host "==> installed: $Dest\mixly.exe"
