# Builds a self-contained Windows folder (and a zip) of SlydeCode.
#
# The executables do not embed the UI: they read `ui/` and `assets/` from beside
# themselves, so the folder is the unit to hand out, not a single .exe.
#
#   dist\package\SlydeCode\
#     slydecode.exe   what a user double-clicks (no console window)
#     slyde.exe       command line and MCP server
#     ui\  assets\  LICENSE
#   dist\SlydeCode-<version>-windows-x64.zip
#
# The GNU target is the verified one on the development machine (the MSVC target
# needs the Windows SDK to link). Pass -Target x86_64-pc-windows-msvc where it is
# installed.
#
# Usage: .\scripts\package-windows.ps1 [-Target <triple>] [-SkipBuild]

param(
    [string]$Target = 'x86_64-pc-windows-gnu',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $SkipBuild) {
    # Windows PowerShell 5.1 turns cargo's progress output on stderr into errors
    # under 'Stop'; only the exit code says whether the build worked.
    $ErrorActionPreference = 'Continue'
    & cargo "+stable-$Target" build --release -p slyde-app --target $Target
    $built = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($built -ne 0) { throw "release build failed" }
}

$release = Join-Path $root "target\$Target\release"
foreach ($name in 'slydecode.exe', 'slyde.exe') {
    if (-not (Test-Path (Join-Path $release $name))) {
        throw "$name not found in $release. Run without -SkipBuild."
    }
}

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
# Staged under dist\package so a copy of the app that is still running from an
# earlier package (its .exe is locked) never blocks the next build.
$out = Join-Path $root 'dist\package\SlydeCode'
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null

Copy-Item (Join-Path $release 'slydecode.exe') $out
Copy-Item (Join-Path $release 'slyde.exe') $out
Copy-Item (Join-Path $root 'ui') (Join-Path $out 'ui') -Recurse
Copy-Item (Join-Path $root 'assets') (Join-Path $out 'assets') -Recurse
Copy-Item (Join-Path $root 'LICENSE') $out

# Never ship local state that happens to live in those folders.
Get-ChildItem $out -Recurse -Include '*.log', '*.slyde', 'runtime.json' | Remove-Item -Force

$zip = Join-Path $root "dist\SlydeCode-$version-windows-x64.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip

$size = '{0:N1}' -f ((Get-Item $zip).Length / 1MB)
Write-Host "Packaged $out"
Write-Host "Zip: $zip ($size MB)"
