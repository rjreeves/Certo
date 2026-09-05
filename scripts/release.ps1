# scripts/release.ps1
# Build all Certo binaries and stage them to dist/.
#
# Usage (from repo root):
#   .\scripts\release.ps1                    # release build (default)
#   .\scripts\release.ps1 -Profile debug     # fast debug build, for local iteration
#   .\scripts\release.ps1 -verbose

param(
    [ValidateSet("debug", "release")]
    [string]$Profile = "release",

    [switch]$verbose
)

Set-Location "$PSScriptRoot\.."
$ErrorActionPreference = "Stop"

$bins = @(
    "certo",
    "certo-test",
    "certo-fmt",
    "certo-lsp",
    "certo-llvm",
    "certo-wasm",
    "certo-ffi",
    "certo-ui"
)

$ext     = if ($IsWindows -or $env:OS -eq "Windows_NT") { ".exe" } else { "" }
$srcDir  = "target\$Profile"
$destDir = "dist"

# ── Build ────────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Building Certo ($Profile)" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

$cargoArgs = @("build", "--bins")
if ($Profile -eq "release") { $cargoArgs += "--release" }
if (-not $verbose) { $cargoArgs += "--quiet" }

cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host "cargo build failed (exit $LASTEXITCODE)" -ForegroundColor Red
    exit $LASTEXITCODE
}

# ── Stage ────────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Staging binaries → $destDir\" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

New-Item -ItemType Directory -Force $destDir | Out-Null

$ok   = 0
$skip = 0

foreach ($bin in $bins) {
    $src  = "$srcDir\$bin$ext"
    $dest = "$destDir\$bin$ext"

    if (-not (Test-Path $src)) {
        Write-Host "  MISSING  $src" -ForegroundColor Yellow
        $skip++
        continue
    }

    try {
        Copy-Item $src $dest -Force
        if ($verbose) { Write-Host "  OK       $bin$ext" -ForegroundColor Green }
        $ok++
    } catch {
        Write-Host "  SKIP     $bin$ext  ($_)" -ForegroundColor Yellow
        $skip++
    }
}

# ── Summary ──────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "Staged $ok/$($bins.Count) binaries to $destDir\" -ForegroundColor $(if ($skip -eq 0) { "Green" } else { "Yellow" })

Write-Host ""
Get-ChildItem "$destDir\*$ext" |
    Sort-Object Name |
    Select-Object Name, @{N="KB"; E={ [math]::Round($_.Length / 1KB, 0) }} |
    Format-Table -AutoSize |
    Out-String |
    Write-Host
