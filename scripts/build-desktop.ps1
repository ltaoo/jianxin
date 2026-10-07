$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --locked --release -p jianxin-cli -p jianxin-desktop
if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed' }
Write-Output 'Built target/release/jianxin.exe and target/release/jianxin-desktop.exe'
Write-Output 'Start the tray: target/release/jianxin.exe tray'
