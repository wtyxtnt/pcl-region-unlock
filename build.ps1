# Build the dwmapi.dll proxy. Optionally deploy it to the launcher directory.
#
# Usage:
#   .\build.ps1
#   .\build.ps1 -Install "D:\PCL"
#
# Note: keep this file ASCII-only. Windows PowerShell 5.1 mis-parses UTF-8
# comments without BOM, which breaks the whole script.

param(
    [string]$Install = "",
    [switch]$Clean
)

$root = $PSScriptRoot
$native = Join-Path $root "native"

# cargo writes warnings to stderr, which PowerShell treats as an error.
# Wrap it in cmd and rely on the exit code only.
function Invoke-Cargo([string]$Arguments) {
    cmd /c "cargo $Arguments"
    if ($LASTEXITCODE -ne 0) { throw "cargo failed: $Arguments" }
}

if ($Clean) {
    Push-Location $native
    try { Invoke-Cargo "clean" } finally { Pop-Location }
}

Push-Location $native
try { Invoke-Cargo "build --release --offline" } finally { Pop-Location }

$dll = Join-Path $native "target\release\dwmapi.dll"
if (-not (Test-Path $dll)) { throw "artifact missing: $dll" }
$size = [math]::Round((Get-Item $dll).Length / 1KB, 1)
Write-Host "built: $dll ($size KB)" -ForegroundColor Green

if ($Install) {
    if (-not (Test-Path $Install)) { throw "target directory not found: $Install" }
    $target = Join-Path $Install "dwmapi.dll"
    Copy-Item $dll $target -Force
    Write-Host "deployed: $target" -ForegroundColor Green
    Write-Host "Start the launcher, then check %TEMP%\pcl-region-unlock.log for 'install finished'."
}
