$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --locked --release -p jianxin-cli -p jianxin-desktop
if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed' }
$executables = @('target/release/jianxin.exe', 'target/release/jianxin-desktop.exe')
foreach ($executable in $executables) {
    $file = Get-Item -LiteralPath $executable
    Write-Output ('Built {0}: {1:N2} MiB' -f $executable, ($file.Length / 1MB))
}
$archive = 'target/release/jianxin-windows.zip'
Compress-Archive -LiteralPath $executables -DestinationPath $archive -CompressionLevel Optimal -Force
Write-Output ('Packaged {0}: {1:N2} MiB' -f $archive, ((Get-Item -LiteralPath $archive).Length / 1MB))
Write-Output 'Start the tray: double-click either exe, or run target/release/jianxin.exe tray'
