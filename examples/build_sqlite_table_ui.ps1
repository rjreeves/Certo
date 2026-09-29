$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$bridge = Join-Path $PSScriptRoot 'sqlite_table_bridge.c'
$object = Join-Path $PSScriptRoot 'out\sqlite_table_bridge.obj'
$source = Join-Path $PSScriptRoot 'sqlite_table_ui.cto'
$output = Join-Path $PSScriptRoot 'out\sqlite-table-ui.exe'
$sqlite = 'C:\Users\robert\Desktop\vicki\target\release\build\libsqlite3-sys-9bdbc71527ac817d\out\sqlite3.lib'
$sqliteInclude = 'C:\Users\robert\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\libsqlite3-sys-0.30.1\sqlite3'
$certo = Join-Path $root 'target\release\certo.exe'

New-Item -ItemType Directory -Force (Split-Path -Parent $object) | Out-Null
& 'C:\Program Files\LLVM\bin\clang.exe' -c $bridge -o $object -O2 "-I$sqliteInclude"
if ($LASTEXITCODE -ne 0) { throw 'Could not compile the SQLite bridge.' }

& $certo build $source -o $output --link $object --link $sqlite
if ($LASTEXITCODE -ne 0) { throw 'Could not compile the Certo viewer.' }

Write-Host "Built $output"
