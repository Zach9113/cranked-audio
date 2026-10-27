# Builds the experimental Windows package (the Windows counterpart of package-release.sh), on Windows with the MSVC
# toolchain: dist\cranked-audio-<tag>-windows-x86_64-experimental.zip with both .exe files and install.ps1, and its
# .sha256 (sha256sum format).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1 [-BuildOnly | -PackageOnly]
#   -BuildOnly    only build the .exe files (so CI can sign them before they're zipped)
#   -PackageOnly  only zip the .exe files already in target\release
param([switch]$BuildOnly, [switch]$PackageOnly)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\.."
$metadata = cargo metadata --manifest-path "$root\Cargo.toml" --format-version 1 --no-deps | ConvertFrom-Json
$version = ($metadata.packages | Where-Object { $_.name -eq 'cranked-audiod' }).version
$name = "cranked-audio-v$version-windows-x86_64-experimental"
$dist = Join-Path $root 'dist'
$package = Join-Path $dist $name

if (-not $PackageOnly) {
    Write-Output "Building $name..."
    # RUSTFLAGS replaces .cargo/config.toml's flags, so the static C runtime is asked for here too
    $env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$root=."
    cargo build --release --locked --manifest-path "$root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
if ($BuildOnly) { Write-Output 'Built target\release\cranked-audiod.exe and cranked-audio.exe'; exit 0 }

Remove-Item -Recurse -Force $package, "$package.zip", "$package.zip.sha256" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $package | Out-Null
Copy-Item "$root\target\release\cranked-audiod.exe", "$root\target\release\cranked-audio.exe", "$root\install.ps1" $package
@"
Cranked Audio $version for Windows: an experimental preview (Cranked Audio is a Linux app first).

Install for your user (Start menu entry, starts at sign-in, no administrator needed):
    powershell -ExecutionPolicy Bypass -File install.ps1
Remove it again (your settings stay in %APPDATA%\cranked-audio):
    powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall

Works: per-app volume through Windows' volume mixer, the USB mixer, MIDI controllers and deej boxes, keyboard
knobs, Auto-lower, media buttons, popups, keyboard shortcuts, the tray icon and installing updates from the app.
Linux-only: the D-Bus API and the panel widget.
https://github.com/Zach9113/cranked-audio
"@ | Set-Content "$package\README.txt"

Compress-Archive -Path $package -DestinationPath "$package.zip"
$hash = (Get-FileHash -Algorithm SHA256 "$package.zip").Hash.ToLower()
# no BOM and LF, like sha256sum writes it
[IO.File]::WriteAllText("$package.zip.sha256", "$hash  $name.zip`n")
Write-Output "Done: dist\$name.zip"
