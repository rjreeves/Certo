param(
    [string]$CertoExe = (Join-Path $PSScriptRoot "..\..\dist\certo.exe"),
    [string]$WispRoot = "C:\Users\robert\Desktop\wisp",
    [string]$OutDir = (Join-Path $PSScriptRoot "out")
)
$ErrorActionPreference = "Stop"
$wispLib = Join-Path $WispRoot "target\release\wisp.dll.lib"
$wispDll = Join-Path $WispRoot "target\release\wisp.dll"
if (-not (Test-Path $wispLib) -or -not (Test-Path $wispDll)) { throw "Wisp release artifacts not found under $WispRoot\target\release" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
& $CertoExe build (Join-Path $PSScriptRoot "luncher2.cto") --link $wispLib --windows-gui -o (Join-Path $OutDir "luncher2.exe")
if ($LASTEXITCODE -ne 0) { throw "Luncher2 build failed ($LASTEXITCODE)" }
Copy-Item $wispDll $OutDir -Force
Copy-Item (Join-Path $PSScriptRoot "luncher2.json") $OutDir -Force
Write-Host "Built $OutDir\luncher2.exe"
