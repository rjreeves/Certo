param(
    [string]$WispRoot = "C:\Users\robert\Desktop\wisp",
    [string]$OutDir = "out\syntra-installer"
)

$ErrorActionPreference = "Stop"

$certoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$certoExe = Join-Path $certoRoot "target\release\certo.exe"
$wispImportLibrary = Join-Path $WispRoot "target\release\wisp.dll.lib"
$wispDll = Join-Path $WispRoot "target\release\wisp.dll"
$outputRoot = Join-Path $certoRoot $OutDir

if (-not (Test-Path -LiteralPath $certoExe)) {
    throw "Certo compiler not found at $certoExe. Run cargo build --release first."
}
if (-not (Test-Path -LiteralPath (Join-Path $WispRoot "Cargo.toml"))) {
    throw "Wisp project not found at $WispRoot. Pass -WispRoot with its location."
}

Push-Location $WispRoot
try {
    cargo build --release --features json
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $outputRoot "assets") | Out-Null

& $certoExe build `
    (Join-Path $certoRoot "examples\certo_installer.cto") `
    --link $wispImportLibrary `
    -o (Join-Path $outputRoot "syntra-installer.exe")

Copy-Item -LiteralPath $wispDll -Destination (Join-Path $outputRoot "wisp.dll") -Force
Copy-Item -LiteralPath (Join-Path $WispRoot "examples\assets\Syntra-120-transparent.png") `
    -Destination (Join-Path $outputRoot "assets\syntra-banner.png") -Force

Write-Host "Built Syntra installer UI in $outputRoot"
Write-Host "Run it from that directory so the relative image path resolves."
