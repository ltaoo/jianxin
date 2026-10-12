# Compatibility entrypoint; the shared pipeline preserves all scene overrides.
# Normal Cargo builds use committed assets and do not require Python or image tools.
$ErrorActionPreference = 'Stop'
python (Join-Path $PSScriptRoot 'generate-icons.py') @args
if ($LASTEXITCODE -ne 0) { throw 'Icon generation failed' }
