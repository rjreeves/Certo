<#
.SYNOPSIS
  Build certo_capi for one platform and stage it where the NuGet project picks it up.

.EXAMPLE
  pwsh packaging/dotnet/stage-native.ps1                      # this machine
  pwsh packaging/dotnet/stage-native.ps1 -Rid osx-x64 -Target x86_64-apple-darwin
#>
param(
    [string]$Rid,
    [string]$Target
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')

if (-not $Rid) {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLower()
    $a = if ($arch -eq 'arm64') { 'arm64' } else { 'x64' }
    if ($IsWindows -or $env:OS -eq 'Windows_NT') { $Rid = "win-$a" }
    elseif ($IsMacOS) { $Rid = "osx-$a" }
    else { $Rid = "linux-$a" }
}

# a separate target dir keeps these flag-specific builds from invalidating normal ones
$env:CARGO_TARGET_DIR = Join-Path $root 'target\pack'
if ($Rid -like 'win-*') {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'   # no VC++ redistributable needed
}

$cargoArgs = @('build', '-p', 'certo-capi', '--release', '--locked')
$outDir = Join-Path $env:CARGO_TARGET_DIR 'release'
if ($Target) {
    $cargoArgs += @('--target', $Target)
    $outDir = Join-Path $env:CARGO_TARGET_DIR "$Target\release"
}
Push-Location $root
try { & cargo @cargoArgs; if ($LASTEXITCODE) { throw "cargo build failed" } } finally { Pop-Location }

$lib = switch -Wildcard ($Rid) {
    'win-*'   { 'certo_capi.dll' }
    'osx-*'   { 'libcerto_capi.dylib' }
    default   { 'libcerto_capi.so' }
}
$dest = Join-Path $PSScriptRoot "Certo.Native\native\$Rid\native"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $outDir $lib) $dest -Force
Write-Host "staged $lib for $Rid -> $dest"
