# Builds the widget, its sensor helper and the installer into .\dist
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

dotnet publish sensors -c Release -p:PublishSingleFile=true -p:DebugType=none -p:Version=$version -o dist
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Pulled in by a dependency, used only on Linux.
Remove-Item dist\*MonoPosixHelper.dll -ErrorAction SilentlyContinue

Copy-Item target\release\thermal-eye.exe, LICENSE, THIRD-PARTY-NOTICES.md dist\ -Force

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found: winget install JRSoftware.InnoSetup' }

# Antivirus can briefly lock the new exe while Inno Setup is writing to it.
foreach ($attempt in 1..3) {
    & $iscc /Q "/DAppVersion=$version" installer\thermal-eye.iss
    if ($LASTEXITCODE -eq 0) { break }
    Start-Sleep -Seconds 5
}
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Get-ChildItem dist | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
