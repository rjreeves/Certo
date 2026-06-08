# examples/run_examples.ps1
# Demonstrates all 8 Certo tools against the example source files.
#
# Run from the repo root:
#   cd examples
#   ..\dist\certo-fmt tasks.certo   (or add dist/ to PATH first)
#
# This script assumes all dist/ binaries are on PATH or in ..\dist\
# Adjust $dist below if needed.

param(
    [string]$dist = "$PSScriptRoot\..\dist"
)

$ErrorActionPreference = "Stop"

function Step([string]$title) {
    Write-Host ""
    Write-Host "=" * 60 -ForegroundColor Cyan
    Write-Host "  $title" -ForegroundColor Cyan
    Write-Host "=" * 60 -ForegroundColor Cyan
}

function Run([string]$cmd) {
    Write-Host "> $cmd" -ForegroundColor DarkGray
    Invoke-Expression $cmd
}

Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force out | Out-Null

# ── 1. certo-fmt ─────────────────────────────────────────────────────────────
Step "1. certo-fmt  — format source files"
Run "$dist\certo-fmt math.certo"
Run "$dist\certo-fmt tasks.certo"
Run "$dist\certo-fmt math_test.certo"

# ── 2. certo (main compiler) ──────────────────────────────────────────────────
Step "2. certo  — compile tasks.certo → native binary"
Run "$dist\certo tasks.certo -o out\tasks.exe"
if (Test-Path out\tasks.exe) {
    Write-Host "Running out\tasks.exe help:" -ForegroundColor Green
    & out\tasks.exe help

    Write-Host ""
    Write-Host "Running out\tasks.exe add 'Buy milk':" -ForegroundColor Green
    & out\tasks.exe add "Buy milk"

    Write-Host ""
    Write-Host "Running out\tasks.exe list:" -ForegroundColor Green
    & out\tasks.exe list

    Write-Host ""
    Write-Host "Running out\tasks.exe count:" -ForegroundColor Green
    & out\tasks.exe count
}

# ── 3. certo-test ────────────────────────────────────────────────────────────
Step "3. certo-test  — run math unit tests"
Run "$dist\certo-test math_test.certo"

# ── 4. certo-llvm ────────────────────────────────────────────────────────────
Step "4. certo-llvm  — emit LLVM IR for math.certo"
Run "$dist\certo-llvm math.certo -o out\math.ll --annotate"
if (Test-Path out\math.ll) {
    Write-Host "First 20 lines of out\math.ll:" -ForegroundColor Green
    Get-Content out\math.ll | Select-Object -First 20
}

# ── 5. certo-wasm ────────────────────────────────────────────────────────────
Step "5. certo-wasm  — emit WASM IR for math.certo (--emit-ir, no toolchain needed)"
Run "$dist\certo-wasm math.certo --emit-ir -o out\math.wasm.ll"
if (Test-Path out\math.wasm.ll) {
    Write-Host "WASM triple in IR:" -ForegroundColor Green
    Select-String "target triple" out\math.wasm.ll | Select-Object -First 1
}

# ── 6. certo-ffi  (C header) ─────────────────────────────────────────────────
Step "6. certo-ffi --header  — generate C header from math.certo pub fns"
Run "$dist\certo-ffi --header math.certo -o out\math.h"
if (Test-Path out\math.h) {
    Write-Host "out\math.h:" -ForegroundColor Green
    Get-Content out\math.h
}

# ── 7. certo-ffi  (REST client) ──────────────────────────────────────────────
Step "7. certo-ffi --rest-client  — generate Certo client from api_schema.json"
Run "$dist\certo-ffi --rest-client api_schema.json -o out\TaskApi.certo"
if (Test-Path out\TaskApi.certo) {
    Write-Host "out\TaskApi.certo:" -ForegroundColor Green
    Get-Content out\TaskApi.certo
}

# ── 8. certo-ui ──────────────────────────────────────────────────────────────
Step "8. certo-ui  — compile views.certo to Htmx HTML"
Run "$dist\certo-ui views.certo -o out\"
Write-Host "Generated HTML files:" -ForegroundColor Green
Get-ChildItem out\*.html | ForEach-Object {
    Write-Host "  $($_.Name)  ($([math]::Round($_.Length/1KB,1)) KB)"
}

# ── 9. certo-lsp ─────────────────────────────────────────────────────────────
Step "9. certo-lsp  — language server (just show version/help; runs as a daemon)"
Run "$dist\certo-lsp --help 2>&1 || true"

# ── Summary ───────────────────────────────────────────────────────────────────
Step "Done — output files in examples\out\"
Get-ChildItem out\ | Select-Object Name, @{N='KB';E={[math]::Round($_.Length/1KB,1)}} |
    Format-Table -AutoSize
