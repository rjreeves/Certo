@echo off
rem Builds not-ide into .\publish\NotIde.exe (framework-dependent, needs the .NET 8+ Desktop Runtime).
cd /d "%~dp0"
dotnet publish NotIde.csproj -c Release -o publish --no-self-contained
if errorlevel 1 (echo Build failed. & exit /b 1)
echo.
echo Built: %~dp0publish\NotIde.exe
