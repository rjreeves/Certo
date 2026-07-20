# examples/run_examples.ps1
# Demonstrates all Certo tools against the example source files.
#
# Prerequisites:
#   - All dist/ binaries built:  cargo build --release  (from repo root)
#   - Clang installed:           winget install LLVM.LLVM
#     (or gcc/cl on PATH — the compiler is auto-detected)
#
# Run from the repo root:
#   cd examples
#   .\run_examples.ps1
# Or pass a custom dist path:
#   .\run_examples.ps1 -dist C:\my\dist

param(
    [string]$dist = "$PSScriptRoot\..\dist"
)

$ErrorActionPreference = "Stop"

function Step([string]$title) {
    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host "  $title" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan
}

function Run([string]$cmd) {
    Write-Host "> $cmd" -ForegroundColor DarkGray
    Invoke-Expression $cmd
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        Write-Host "  [exit $LASTEXITCODE]" -ForegroundColor Yellow
    }
}

Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force out | Out-Null

# ── 1. certo-fmt ─────────────────────────────────────────────────────────────
Step "1. certo-fmt  — format source files"
Run "$dist\certo-fmt math.cto"
Run "$dist\certo-fmt tasks.cto"
Run "$dist\certo-fmt math_test.cto"

# ── 2. certo (compiler) — executable ─────────────────────────────────────────
Step "2. certo  — compile tasks.cto to a native executable"
Run "$dist\certo tasks.cto -o out\tasks.exe -v"

if (Test-Path out\tasks.exe) {
    Write-Host ""
    Write-Host "Running tasks.exe (no args = help):" -ForegroundColor Green
    & out\tasks.exe

    Write-Host ""
    Write-Host "Running tasks.exe list:" -ForegroundColor Green
    & out\tasks.exe list
}

# ── 3. certo (compiler) — shared library / DLL ───────────────────────────────
Step "3. certo --emit-dll  — compile math.cto to a shared library"
Run "$dist\certo math.cto --emit-dll -o out\math.dll -v"

if (Test-Path out\math.dll) {
    Write-Host ""
    Write-Host "Generated:" -ForegroundColor Green
    Get-Item out\math.dll, out\math.lib -ErrorAction SilentlyContinue |
        Select-Object Name, @{N='KB';E={[math]::Round($_.Length/1KB,1)}} |
        Format-Table -AutoSize

    Write-Host "Exported symbols:" -ForegroundColor Green
    $llvmObjdump = "C:\Program Files\LLVM\bin\llvm-objdump.exe"
    if (Test-Path $llvmObjdump) {
        & $llvmObjdump -p out\math.dll | Select-String "certo_"
    }
}

# ── 4. certo-test ────────────────────────────────────────────────────────────
Step "4. certo-test  — run math unit tests"
Run "$dist\certo-test math_test.cto"

# ── 5. certo-llvm ────────────────────────────────────────────────────────────
Step "5. certo-llvm  — emit LLVM IR for math.cto"
Run "$dist\certo-llvm math.cto -o out\math.ll --annotate"
if (Test-Path out\math.ll) {
    Write-Host "First 20 lines of out\math.ll:" -ForegroundColor Green
    Get-Content out\math.ll | Select-Object -First 20
}

# ── 6. certo-wasm ────────────────────────────────────────────────────────────
Step "6. certo-wasm  — emit WASM IR for math.cto"
Run "$dist\certo-wasm math.cto --emit-ir -o out\math.wasm.ll"
if (Test-Path out\math.wasm.ll) {
    Write-Host "WASM target triple:" -ForegroundColor Green
    Select-String "target triple" out\math.wasm.ll | Select-Object -First 1
}

# ── 7. certo-ffi (C header) ───────────────────────────────────────────────────
Step "7. certo-ffi --header  — generate C header from math.cto pub fns"
Run "$dist\certo-ffi --header math.cto -o out\math.h"
if (Test-Path out\math.h) {
    Write-Host "out\math.h:" -ForegroundColor Green
    Get-Content out\math.h
}

# ── 8. certo-ffi (REST client) ────────────────────────────────────────────────
Step "8. certo-ffi --rest-client  — generate Certo client from api_schema.json"
Run "$dist\certo-ffi --rest-client api_schema.json -o out\TaskApi.cto"
if (Test-Path out\TaskApi.cto) {
    Write-Host "out\TaskApi.cto:" -ForegroundColor Green
    Get-Content out\TaskApi.cto
}

# ── 9. certo-ui ───────────────────────────────────────────────────────────────
Step "9. certo-ui  — compile views.cto to Htmx HTML"
Run "$dist\certo-ui views.cto -o out\"
Write-Host "Generated HTML files:" -ForegroundColor Green
Get-ChildItem out\*.html -ErrorAction SilentlyContinue |
    Select-Object Name, @{N='KB';E={[math]::Round($_.Length/1KB,1)}} |
    Format-Table -AutoSize

# ── 10. certo-lsp ─────────────────────────────────────────────────────────────
Step "10. certo-lsp  — language server help (runs as daemon in normal use)"
& "$dist\certo-lsp" --help 2>&1 | Select-Object -First 10

# ── Summary ───────────────────────────────────────────────────────────────────
Step "Done — output files in examples\out\"
Get-ChildItem out\ -ErrorAction SilentlyContinue |
    Select-Object Name, @{N='KB';E={[math]::Round($_.Length/1KB,1)}} |
    Sort-Object Name |
    Format-Table -AutoSize
