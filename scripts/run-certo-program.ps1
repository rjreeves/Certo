# Validate, compile, and run a Certo program through the native toolchain.
#
# Examples:
#   .\scripts\run-certo-program.ps1 examples\tasks.cto
#   .\scripts\run-certo-program.ps1 src\main.cto -ProgramArguments @("--port", "8080")
#   .\scripts\run-certo-program.ps1 src\main.cto -BuildOnly -Profile release

[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$Program,

    [ValidateSet("debug", "release")]
    [string]$Profile = "debug",

    [string]$OutputDirectory = "",

    [string[]]$ProgramArguments = @(),

    [switch]$SkipCompilerBuild,

    [switch]$SkipFormatCheck,

    [switch]$SkipLint,

    [switch]$SkipTests,

    [switch]$BuildOnly,

    [switch]$KeepGoing
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$programPath = [System.IO.Path]::GetFullPath((Resolve-Path -LiteralPath $Program).Path)
if ([System.IO.Path]::GetExtension($programPath) -ne ".cto") {
    throw "Program must be a .cto file: $programPath"
}

$programDirectory = Split-Path -Parent $programPath
$programName = [System.IO.Path]::GetFileNameWithoutExtension($programPath)
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "out\programs\$programName"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot $OutputDirectory
}
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)

$extension = if ($IsWindows -or $env:OS -eq "Windows_NT") { ".exe" } else { "" }
$compiler = Join-Path $repoRoot "target\$Profile\certo$extension"
$executable = Join-Path $OutputDirectory "$programName$extension"
$results = [System.Collections.Generic.List[object]]::new()
$failed = $false

function Invoke-Stage {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [string]$Command,
        [string[]]$CommandArgs = @(),
        [string]$WorkingDirectory = $repoRoot
    )

    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
    $shownArgs = $CommandArgs | ForEach-Object {
        if ($_ -match '\s') { '"' + $_ + '"' } else { $_ }
    }
    Write-Host ("> {0} {1}" -f $Command, ($shownArgs -join " ")).TrimEnd() -ForegroundColor DarkGray

    $started = Get-Date
    Push-Location $WorkingDirectory
    try {
        & $Command @CommandArgs
        $exitCode = if ($null -eq $LASTEXITCODE) { 0 } else { $LASTEXITCODE }
    } finally {
        Pop-Location
    }

    $results.Add([pscustomobject]@{
        Stage = $Name
        Result = if ($exitCode -eq 0) { "PASS" } else { "FAIL ($exitCode)" }
        Seconds = [math]::Round(((Get-Date) - $started).TotalSeconds, 2)
    })

    if ($exitCode -ne 0) {
        $script:failed = $true
        if (-not $KeepGoing) {
            throw "Stage '$Name' failed with exit code $exitCode."
        }
        return $false
    }
    return $true
}

try {
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

    if (-not $SkipCompilerBuild) {
        $cargoArgs = @("build", "--package", "certo", "--bin", "certo")
        if ($Profile -eq "release") { $cargoArgs += "--release" }
        [void](Invoke-Stage "Build Certo compiler ($Profile)" "cargo" $cargoArgs)
    }

    if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
        throw "Certo compiler not found: $compiler. Run without -SkipCompilerBuild."
    }

    [void](Invoke-Stage "Compiler version" $compiler @("--version"))

    if (-not $SkipFormatCheck) {
        [void](Invoke-Stage "Check formatting" $compiler @("fmt", "--check", $programPath))
    }

    [void](Invoke-Stage "Parse and type-check" $compiler @("check", $programPath))

    if (-not $SkipLint) {
        [void](Invoke-Stage "Lint program" $compiler @("lint", $programPath))
    }

    if (-not $SkipTests) {
        [void](Invoke-Stage "Run embedded tests" $compiler @("test", $programPath))
    }

    $buildOk = Invoke-Stage "Compile native program" $compiler @(
        "build", $programPath, "-o", $executable
    )

    if (-not $BuildOnly -and $buildOk -and (Test-Path -LiteralPath $executable -PathType Leaf)) {
        [void](Invoke-Stage "Run $programName" $executable $ProgramArguments $programDirectory)
    }
} catch {
    $failed = $true
    Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""
Write-Host "Program toolchain summary" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host

if ($failed) { exit 1 }

if ($BuildOnly) {
    Write-Host "Build complete: $executable" -ForegroundColor Green
} else {
    Write-Host "Program completed successfully: $executable" -ForegroundColor Green
}
exit 0
