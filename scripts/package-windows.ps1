# Builds the experimental Windows package (the Windows counterpart of package-release.sh), on Windows with the MSVC
# toolchain: dist\cranked-audio-<tag>-windows-x86_64-experimental.zip with both .exe files and install.ps1, and its
# .sha256 (sha256sum format).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\.."
$metadata = cargo metadata --manifest-path "$root\Cargo.toml" --format-version 1 --no-deps | ConvertFrom-Json
$version = ($metadata.packages | Where-Object { $_.name -eq 'cranked-audiod' }).version
$name = "cranked-audio-v$version-windows-x86_64-experimental"
$dist = Join-Path $root 'dist'
$package = Join-Path $dist $name

Write-Output "Building $name..."
# RUSTFLAGS replaces .cargo/config.toml's flags, so the static C runtime is asked for here too
$env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$root=."
cargo build --release --locked --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

Remove-Item -Recurse -Force $package, "$package.zip", "$package.zip.sha256" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $package | Out-Null
Copy-Item "$root\target\release\cranked-audiod.exe", "$root\target\release\cranked-audio.exe", "$root\install.ps1" $package
@"
Cranked Audio $version for Windows: an experimental preview (Cranked Audio is a Linux app first).

Install for your user (Start menu entry, starts at sign-in, no administrator needed):
    powershell -ExecutionPolicy Bypass -File install.ps1
Remove it again (your settings stay in %APPDATA%\cranked-audio):
    powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall

Works: per-app volume through Windows' volume mixer, the USB mixer, MIDI controllers and deej boxes, Auto-lower,
media keys and the tray icon. Linux-only for now: popups, keyboard shortcuts, keyboard knobs, the D-Bus API, the
panel widget and installing updates from the app.
https://github.com/Zach9113/cranked-audio
"@ | Set-Content "$package\README.txt"

Compress-Archive -Path $package -DestinationPath "$package.zip"
$hash = (Get-FileHash -Algorithm SHA256 "$package.zip").Hash.ToLower()
# no BOM and LF, like sha256sum writes it
[IO.File]::WriteAllText("$package.zip.sha256", "$hash  $name.zip`n")
Write-Output "Done: dist\$name.zip"
