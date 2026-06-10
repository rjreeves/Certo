# scripts/dist-package.ps1
# Package release binaries + docs + examples into a distributable archive.
#
# Usage (from repo root):
#   .\scripts\dist-package.ps1 -version "0.1.0"
#   .\scripts\dist-package.ps1 -version "0.1.0" -out C:\releases
#   .\scripts\dist-package.ps1 -version "0.1.0" -rebuild

param(
    [Parameter(Mandatory)]
    [string]$version,

    [string]$out = "dist",

    # Run scripts\release.ps1 first so binaries are up-to-date.
    [switch]$rebuild
)

Set-Location "$PSScriptRoot\.."
$ErrorActionPreference = "Stop"

$isWindows = $IsWindows -or $env:OS -eq "Windows_NT"
$ext       = if ($isWindows) { ".exe" } else { "" }
$platform  = if ($isWindows) {
    "windows-x64"
} elseif ($IsMacOS) {
    "macos-arm64"
} else {
    "linux-x64"
}

$archiveName = "certo-$version-$platform"
$stagingDir  = "$env:TEMP\$archiveName"

# ── Optionally rebuild first ─────────────────────────────────────────────────
if ($rebuild) {
    Write-Host "Rebuilding binaries…" -ForegroundColor Cyan
    & "$PSScriptRoot\release.ps1"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# ── Validate dist/ binaries exist ────────────────────────────────────────────
$bins = @("certo","certo-test","certo-fmt","certo-lsp","certo-llvm","certo-wasm","certo-ffi","certo-ui")

foreach ($bin in $bins) {
    if (-not (Test-Path "dist\$bin$ext")) {
        Write-Host "ERROR: dist\$bin$ext not found — run .\scripts\release.ps1 first." -ForegroundColor Red
        exit 1
    }
}

# ── Build staging directory ───────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Staging → $stagingDir" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

if (Test-Path $stagingDir) { Remove-Item $stagingDir -Recurse -Force }
New-Item -ItemType Directory -Force $stagingDir | Out-Null

# Binaries
foreach ($bin in $bins) {
    Copy-Item "dist\$bin$ext" "$stagingDir\$bin$ext" -Force
}
if (-not $isWindows) {
    # Make binaries executable on Unix
    foreach ($bin in $bins) { chmod +x "$stagingDir/$bin" }
}

# Docs
$docsOut = "$stagingDir\docs"
New-Item -ItemType Directory -Force $docsOut | Out-Null
Copy-Item "docs\GUIDE.md" "$docsOut\GUIDE.md" -Force
if (Test-Path "docs\Certo_Language_Specification.docx") {
    Copy-Item "docs\Certo_Language_Specification.docx" "$docsOut\" -Force
}

# Examples (source files only — no out/ artefacts)
$examplesOut = "$stagingDir\examples"
New-Item -ItemType Directory -Force $examplesOut | Out-Null
Get-ChildItem "examples\*.cto" | Copy-Item -Destination $examplesOut
if (Test-Path "examples\api_schema.json") {
    Copy-Item "examples\api_schema.json" "$examplesOut\" -Force
}
if (Test-Path "examples\run_examples.ps1") {
    Copy-Item "examples\run_examples.ps1" "$examplesOut\" -Force
}

# README
if (Test-Path "README.md") {
    Copy-Item "README.md" "$stagingDir\README.md" -Force
}

# Version stamp
"$version" | Out-File "$stagingDir\VERSION" -Encoding utf8 -NoNewline

# ── Create archive ────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 60) -ForegroundColor Cyan
Write-Host "  Creating archive" -ForegroundColor Cyan
Write-Host ("=" * 60) -ForegroundColor Cyan

New-Item -ItemType Directory -Force $out | Out-Null

if ($isWindows) {
    $archivePath = "$out\$archiveName.zip"
    if (Test-Path $archivePath) { Remove-Item $archivePath -Force }
    Compress-Archive -Path "$stagingDir\*" -DestinationPath $archivePath
    $size = [math]::Round((Get-Item $archivePath).Length / 1MB, 2)
    Write-Host ""
    Write-Host "Created: $archivePath  ($size MB)" -ForegroundColor Green
} else {
    $archivePath = "$out/$archiveName.tar.gz"
    tar -czf $archivePath -C $env:TEMP $archiveName
    $size = [math]::Round((Get-Item $archivePath).Length / 1MB, 2)
    Write-Host ""
    Write-Host "Created: $archivePath  ($size MB)" -ForegroundColor Green
}

# ── Cleanup ───────────────────────────────────────────────────────────────────
Remove-Item $stagingDir -Recurse -Force

# ── Contents summary ─────────────────────────────────────────────────────────
Write-Host ""
Write-Host "Archive contents:" -ForegroundColor Cyan
if ($isWindows) {
    (Get-ChildItem $archivePath | ForEach-Object {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [System.IO.Compression.ZipFile]::OpenRead($_.FullName).Entries |
            Select-Object FullName, @{N="KB"; E={ [math]::Round($_.Length/1KB,1) }}
    }) | Format-Table -AutoSize
}
