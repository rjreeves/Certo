# Run a reproducible smoke test of the complete Certo toolchain.
#
# The script uses binaries directly from target/<profile>, never modifies the
# example sources, and writes generated artifacts beneath out/toolchain.

[CmdletBinding()]
param(
    [ValidateSet("debug", "release")]
    [string]$Profile = "debug",

    [string]$OutputDirectory = "",

    [switch]$SkipBuild,

    [switch]$SkipRustTests,

    [switch]$KeepGoing
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "out\toolchain"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot $OutputDirectory
}
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)

$binaryExtension = if ($IsWindows -or $env:OS -eq "Windows_NT") { ".exe" } else { "" }
$binaryDirectory = Join-Path $repoRoot "target\$Profile"
$results = [System.Collections.Generic.List[object]]::new()
$failed = $false

function Write-Stage([string]$Name) {
    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
}

function Invoke-ToolchainStep {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [string]$Command,
        [string[]]$CommandArgs = @()
    )

    Write-Stage $Name
    $displayArgs = $CommandArgs | ForEach-Object {
        if ($_ -match '\s') { '"' + $_ + '"' } else { $_ }
    }
    Write-Host ("> {0} {1}" -f $Command, ($displayArgs -join " ")).TrimEnd() -ForegroundColor DarkGray

    $started = Get-Date
    & $Command @CommandArgs
    $exitCode = if ($null -eq $LASTEXITCODE) { 0 } else { $LASTEXITCODE }
    $elapsed = (Get-Date) - $started
    $script:results.Add([pscustomobject]@{
        Step = $Name
        Result = if ($exitCode -eq 0) { "PASS" } else { "FAIL ($exitCode)" }
        Seconds = [math]::Round($elapsed.TotalSeconds, 2)
    })

    if ($exitCode -ne 0) {
        $script:failed = $true
        if (-not $KeepGoing) {
            throw "Step '$Name' failed with exit code $exitCode."
        }
    }
}

function Get-CertoBinary([string]$Name) {
    $path = Join-Path $binaryDirectory "$Name$binaryExtension"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required binary not found: $path. Run without -SkipBuild first."
    }
    return $path
}

Push-Location $repoRoot
try {
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $uiOutput = Join-Path $OutputDirectory "ui"
    New-Item -ItemType Directory -Force -Path $uiOutput | Out-Null

    if (-not $SkipBuild) {
        $cargoBuildArgs = @("build", "--workspace", "--bins")
        if ($Profile -eq "release") { $cargoBuildArgs += "--release" }
        Invoke-ToolchainStep "Build Rust toolchain ($Profile)" "cargo" $cargoBuildArgs
    }

    if (-not $SkipRustTests) {
        $cargoTestArgs = @("test", "--workspace")
        if ($Profile -eq "release") { $cargoTestArgs += "--release" }
        Invoke-ToolchainStep "Run Rust workspace tests" "cargo" $cargoTestArgs
    }

    $certo = Get-CertoBinary "certo"
    $certoFmt = Get-CertoBinary "certo-fmt"
    $certoTest = Get-CertoBinary "certo-test"
    $certoLlvm = Get-CertoBinary "certo-llvm"
    $certoWasm = Get-CertoBinary "certo-wasm"
    $certoFfi = Get-CertoBinary "certo-ffi"
    $certoUi = Get-CertoBinary "certo-ui"
    [void](Get-CertoBinary "certo-lsp") # Built and presence-checked; it speaks stdio and is not launched here.

    Invoke-ToolchainStep "Report compiler version" $certo @("--version")
    # Use the canonical, already-formatted library sample. Some UI/test fixtures
    # intentionally retain non-canonical layouts for parser coverage.
    Invoke-ToolchainStep "Check example formatting" $certoFmt @("--check", "examples/math.cto")
    Invoke-ToolchainStep "Type-check Certo source" $certo @("check", "examples/math.cto")
    Invoke-ToolchainStep "Lint Certo source" $certo @("lint", "examples/math.cto")
    Invoke-ToolchainStep "Run Certo tests" $certoTest @("examples/math_test.cto")
    Invoke-ToolchainStep "Compile native executable" $certo @(
        "build", "examples/tasks.cto", "-o", (Join-Path $OutputDirectory "tasks$binaryExtension")
    )
    Invoke-ToolchainStep "Emit native C" $certo @(
        "build", "examples/math.cto", "--emit-c", "-o", (Join-Path $OutputDirectory "math.c")
    )
    Invoke-ToolchainStep "Emit LLVM IR" $certoLlvm @(
        "examples/math.cto", "--annotate", "-o", (Join-Path $OutputDirectory "math.ll")
    )
    Invoke-ToolchainStep "Emit WebAssembly LLVM IR" $certoWasm @(
        "examples/math.cto", "--emit-ir", "--no-color", "-o", (Join-Path $OutputDirectory "math.wasm.ll")
    )
    Invoke-ToolchainStep "Generate C header" $certoFfi @(
        "--header", "examples/math.cto", "-o", (Join-Path $OutputDirectory "math.h")
    )
    Invoke-ToolchainStep "Generate REST client" $certoFfi @(
        "--rest-client", "examples/api_schema.json", "-o", (Join-Path $OutputDirectory "TaskApi.cto")
    )
    Invoke-ToolchainStep "Generate UI HTML" $certoUi @(
        "examples/views.cto", "--html", "--no-color", "-o", $uiOutput
    )
} catch {
    $failed = $true
    Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Toolchain summary" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host

if ($failed) {
    exit 1
}

Write-Host "All toolchain checks passed. Artifacts: $OutputDirectory" -ForegroundColor Green
exit 0
