# Builds the widget and its sensor helper into .\dist
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

dotnet publish sensors -c Release -r win-x64 --self-contained false `
    -p:PublishSingleFile=true -p:DebugType=none -o dist
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Pulled in by a dependency, used only on Linux.
Remove-Item dist\*MonoPosixHelper.dll -ErrorAction SilentlyContinue

Copy-Item target\release\thermal-eye.exe, LICENSE, THIRD-PARTY-NOTICES.md dist\ -Force
Get-ChildItem dist | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
