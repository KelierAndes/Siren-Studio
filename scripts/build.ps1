param(
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    # libopus is compiled from source by opus-head-sys. The CMake and MSVC
    # toolchains ship inside Visual Studio Build Tools but are not on PATH.
    $candidates = @()

    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $install = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if (-not $install) {
            $install = & $vswhere -latest -products * -property installationPath
        }
        if ($install) {
            $cmakeDir = Join-Path $install "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
            if (Test-Path (Join-Path $cmakeDir "cmake.exe")) {
                $candidates += $cmakeDir
            }
            $msvcRoot = Join-Path $install "VC\Tools\MSVC"
            if (Test-Path $msvcRoot) {
                $version = Get-ChildItem $msvcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
                $clDir = Join-Path $version.FullName "bin\Hostx64\x64"
                if (Test-Path (Join-Path $clDir "cl.exe")) {
                    $candidates += $clDir
                }
            }
        }
    }

    $pathCmake = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($pathCmake) {
        $candidates += (Split-Path $pathCmake.Source)
    }
    if ($candidates.Count -gt 0) {
        $env:PATH = (($candidates | Select-Object -Unique) -join ";") + ";" + $env:PATH
    }

    if (-not (Get-Command cmake.exe -ErrorAction SilentlyContinue)) {
        throw "cmake was not found. Install Visual Studio Build Tools with 'Desktop development with C++' (it includes CMake)."
    }
    Write-Host "cmake: $((Get-Command cmake.exe).Source)"

    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    if (-not $SkipTests) {
        cargo test --workspace
        if ($LASTEXITCODE -ne 0) { throw "unit tests failed" }
    }

    Write-Host ""
    Write-Host "Built: $root\target\release\siren.exe"
    Write-Host "End-to-end check: python scripts/verify_e2e.py"
}
finally {
    Pop-Location
}
