# Builds the Flutter Windows console (siren_ui.exe) in release mode.
#
# - Locates MSVC/CMake via vswhere (Cargokit compiles opus-head-sys from
#   source, which needs cmake on PATH).
# - Sets CARGOKIT_TEMP_DIR to a short path: MSBuild fails with FTK1011 when
#   Cargokit try-compiles in the deep default temp directory.
param(
    [switch]$SkipBuild  # Only print the environment that would be used.
)

$ErrorActionPreference = "Stop"
$ui = Join-Path $PSScriptRoot "..\ui-flutter"

# Short-path temp dir for Cargokit (avoids MSBuild FTK1011 long-path errors).
$env:CARGOKIT_TEMP_DIR = "C:/siren_build/cargokit"
New-Item -ItemType Directory -Force -Path $env:CARGOKIT_TEMP_DIR | Out-Null

# The flutter build directory sits on C: via a junction for the same reason.
$buildLink = Join-Path $ui "build"
if (-not (Test-Path $buildLink)) {
    New-Item -ItemType Directory -Force -Path "C:/siren_build/ui-flutter" | Out-Null
    New-Item -ItemType Junction -Path $buildLink -Target "C:/siren_build/ui-flutter" | Out-Null
}

# CMake/MSVC ship inside Visual Studio but are not on PATH.
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $install = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $install) {
        $install = & $vswhere -latest -products * -property installationPath
    }
    if ($install) {
        $cmakeDir = Join-Path $install "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
        if (Test-Path (Join-Path $cmakeDir "cmake.exe")) {
            $env:PATH = "$cmakeDir;$env:PATH"
        }
        $msvcRoot = Join-Path $install "VC\Tools\MSVC"
        if (Test-Path $msvcRoot) {
            $version = Get-ChildItem $msvcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
            $clDir = Join-Path $version.FullName "bin\Hostx64\x64"
            if (Test-Path (Join-Path $clDir "cl.exe")) {
                $env:PATH = "$clDir;$env:PATH"
            }
        }
    }
}

Write-Host "CARGOKIT_TEMP_DIR = $env:CARGOKIT_TEMP_DIR"
if (-not (Get-Command cmake.exe -ErrorAction SilentlyContinue)) {
    Write-Warning "cmake.exe not found on PATH; opus-head-sys may fail to build."
}

if ($SkipBuild) { return }

Push-Location $ui
try {
    # Icon tree-shaking drops glyphs that the shaker cannot see as static
    # IconData references; the symptom is blank buttons/panel icons. Keep the
    # full MaterialIcons font for a release desktop build.
    flutter build windows --release --no-tree-shake-icons
    if ($LASTEXITCODE -ne 0) { throw "flutter build failed" }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Built: $([System.IO.Path]::GetFullPath((Join-Path $ui 'build\windows\x64\runner\Release\siren_ui.exe')))"
