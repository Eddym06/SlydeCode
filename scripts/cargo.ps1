# Runs cargo with the MSVC environment loaded.
#
# Rust on Windows targets `x86_64-pc-windows-msvc` and needs `link.exe`, which is
# only on PATH inside a Visual Studio developer prompt. This wrapper finds the
# installed Build Tools / VS via vswhere, imports the vcvars64 environment and
# then forwards every argument to cargo.
#
# Usage: .\scripts\cargo.ps1 test -p slyde-core

$ErrorActionPreference = 'Continue'

function Get-VsInstallPath {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $path = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($path) { return $path.Trim() }
    }
    foreach ($candidate in @(
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\Community",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\BuildTools"
    )) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

# A machine without the C++ build tools cannot link the MSVC target at all
# (`LNK1181: kernel32.lib`). Rather than failing, fall through to the GNU
# toolchain, which is the verified target for this project on Windows.
$gnuToolchain = "stable-x86_64-pc-windows-gnu"
$useGnu = $false

if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    $vs = Get-VsInstallPath
    if (-not $vs) {
        $gnu = (& rustup toolchain list 2>$null) | Select-String -SimpleMatch $gnuToolchain
        if (-not $gnu) {
            throw "No C++ build tools and no GNU toolchain. Run: rustup toolchain install $gnuToolchain"
        }
        Write-Host "MSVC build tools not found; using $gnuToolchain" -ForegroundColor DarkYellow
        $useGnu = $true
    }
    $vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $vcvars)) { throw "vcvars64.bat not found under $vs" }

    $envDump = cmd /c "`"$vcvars`" >nul 2>&1 && set"
    foreach ($line in $envDump) {
        if ($line -match '^([^=]+)=(.*)$') {
            Set-Item -Path "env:$($matches[1])" -Value $matches[2]
        }
    }
}

if ($useGnu) {
    & cargo "+$gnuToolchain" @args 2>&1 | ForEach-Object { "$_" }
} else {
    & cargo @args 2>&1 | ForEach-Object { "$_" }
}
exit $LASTEXITCODE
