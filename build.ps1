# build.ps1 — Build Certo compiler + all example programs
# Usage:
#   .\build.ps1            # release build
#   .\build.ps1 -Debug     # debug build (faster compile, larger binaries)
#   .\build.ps1 -NoExamples # skip example compilation

param(
    [switch]$Debug,
    [switch]$NoExamples
)

$ErrorActionPreference = "Stop"
$profile = if ($Debug) { "debug" } else { "release" }

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Ok($msg)   { Write-Host "    $msg" -ForegroundColor Green }
function Fail($msg) { Write-Host "    ERROR: $msg" -ForegroundColor Red; exit 1 }

# ── 1. Bump patch version on release builds ──────────────────────────────────
$cliToml = "$PSScriptRoot\crates\cli\Cargo.toml"
if (-not $Debug) {
    $tomlContent = Get-Content $cliToml -Raw
    if ($tomlContent -match 'version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') {
        $major = [int]$Matches[1]
        $minor = [int]$Matches[2]
        $patch = [int]$Matches[3] + 1
        $newVersion = "$major.$minor.$patch"
        $tomlContent = $tomlContent -replace 'version\s*=\s*"\d+\.\d+\.\d+"', "version = `"$newVersion`""
        Set-Content $cliToml $tomlContent -NoNewline
        Ok "Version bumped to $newVersion"
    } else {
        Write-Host "    WARN: could not parse version from $cliToml" -ForegroundColor Yellow
    }
}

# ── 2. Compile all Rust crates ────────────────────────────────────────────────
Step "Building all Rust crates ($profile)"
if ($Debug) { cargo build } else { cargo build --release }
if ($LASTEXITCODE -ne 0) { Fail "cargo build failed" }
Ok "Rust build complete"

# ── 3. Copy binaries to dist\ ─────────────────────────────────────────────────
Step "Copying binaries to dist\"
New-Item -ItemType Directory -Force -Path dist | Out-Null

$bins = @(
    "certo",
    "certo-ffi",
    "certo-fmt",
    "certo-llvm",
    "certo-lsp",
    "certo-test",
    "certo-ui",
    "certo-wasm"
)

foreach ($bin in $bins) {
    $src = "target\$profile\$bin.exe"
    $dst = "dist\$bin.exe"
    if (Test-Path $src) {
        Copy-Item $src $dst -Force
        $size = (Get-Item $dst).Length / 1KB
        Ok ("{0,-20} -> dist\{1}.exe  ({2:F0} KB)" -f $bin, $bin, $size)
    } else {
        Write-Host "    WARN: $src not found (skipping)" -ForegroundColor Yellow
    }
}

if ($NoExamples) {
    Step "Skipping example builds (-NoExamples)"
    exit 0
}

# ── 4. Compile example programs ───────────────────────────────────────────────
Step "Building example programs"

$certo = "dist\certo.exe"
if (-not (Test-Path $certo)) { Fail "dist\certo.exe not found" }

# Examples known to compile cleanly to standalone executables.
# Library-only modules (math.cto, math_module.cto, views.cto) are excluded —
# build them with: certo build math.cto --emit-dll
$examples = @(
    @{ src = "examples\generic_test.cto"; out = "dist\generic_test.exe" },
    @{ src = "examples\gen_simple.cto";   out = "dist\gen_simple.exe" },
    @{ src = "examples\msgbox.cto";       out = "dist\msgbox.exe" },
    @{ src = "examples\tasks.cto";        out = "dist\tasks.exe" },
    @{ src = "examples\db_users.cto";     out = "dist\db_users.exe" },
    @{ src = "examples\http_server.cto";  out = "dist\http_server.exe" },
    @{ src = "examples\littleQ.cto";      out = "dist\littleQ.exe" },
    @{ src = "examples\pgcheck.cto";      out = "dist\pgcheck.exe" }
)

$ok = 0; $fail = 0
foreach ($ex in $examples) {
    if (-not (Test-Path $ex.src)) {
        Write-Host ("    SKIP  {0}  (file not found)" -f $ex.src) -ForegroundColor Yellow
        continue
    }
    $result = & $certo build $ex.src -o $ex.out 2>&1
    if ($LASTEXITCODE -eq 0) {
        $size = if (Test-Path $ex.out) { "{0:F0} KB" -f ((Get-Item $ex.out).Length / 1KB) } else { "?" }
        Ok ("{0,-35} -> {1}  ({2})" -f $ex.src, $ex.out, $size)
        $ok++
    } else {
        Write-Host ("    FAIL  {0}" -f $ex.src) -ForegroundColor Red
        Write-Host ("          {0}" -f ($result | Select-Object -Last 3 | Out-String).Trim()) -ForegroundColor DarkRed
        $fail++
    }
}

# ── Summary ───────────────────────────────────────────────────────────────────
Step "Done"
Ok "$($bins.Count) compiler binaries in dist\"
if (-not $NoExamples) {
    Ok "$ok example(s) compiled OK"
    if ($fail -gt 0) { Write-Host "    $fail example(s) failed" -ForegroundColor Yellow }
}
