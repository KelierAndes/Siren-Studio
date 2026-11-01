# Builds the Android APK in release mode.
#
# Icon tree-shaking can strip MaterialIcons glyphs that are passed through
# fields/records instead of direct Icon(Icons.x) calls, which shows up as
# blank buttons. Keep the full icon font in the APK.
param()
$ErrorActionPreference = "Stop"
$ui = Join-Path $PSScriptRoot "..\ui-flutter"
Push-Location $ui
try {
    flutter build apk --release --no-tree-shake-icons
    if ($LASTEXITCODE -ne 0) { throw "flutter build apk failed" }
}
finally {
    Pop-Location
}
Write-Host ""
Write-Host "Built: $([System.IO.Path]::GetFullPath((Join-Path $ui 'build\app\outputs\flutter-apk\app-release.apk')))"
