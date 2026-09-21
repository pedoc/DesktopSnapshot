param(
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    npm ci --prefix crates/ui
    cargo build --release --locked -p desktop-snapshot-cli
    npm run tauri build --prefix crates/ui

    $packageName = "DesktopSnapshot-v$Version-portable-windows-x64"
    $stage = Join-Path $root "target\$packageName"
    $outputDir = Join-Path $root "dist"
    $archive = Join-Path $outputDir "$packageName.zip"

    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $stage, $outputDir | Out-Null

    Copy-Item "target\release\desktop-snapshot.exe" $stage
    Copy-Item "target\release\desktop-snapshot-ui.exe" $stage
    Copy-Item "README.md", "LICENSE", "THIRD-PARTY-NOTICES.md" $stage
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $archive -Force

    $hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
    "$hash  $(Split-Path $archive -Leaf)" | Set-Content -LiteralPath (Join-Path $outputDir "SHA256SUMS.txt") -Encoding ascii
    Write-Host "Portable package: $archive"
}
finally {
    Pop-Location
}
