# Installs Cranked Audio on Windows for the current user (no administrator needed):
#   %LOCALAPPDATA%\Programs\Cranked Audio\cranked-audiod.exe + cranked-audio.exe, a Start menu entry, and a start at
#   sign-in (the current user's Run key).
# Usage: powershell -ExecutionPolicy Bypass -File install.ps1              install from the newest build next to it
#        powershell -ExecutionPolicy Bypass -File install.ps1 -From DIR    install the two .exe files found in DIR
#        powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall   stop it and remove everything this installed
#                                                                          (settings in %APPDATA%\cranked-audio stay)
param([string]$From = '', [switch]$Uninstall)
$ErrorActionPreference = 'Stop'

$programs = Join-Path $env:LOCALAPPDATA 'Programs\Cranked Audio'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Cranked Audio.lnk'
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = 'Cranked Audio'

function Stop-Installed {
    # Windows can't replace or delete a running .exe
    Get-Process cranked-audiod, cranked-audio -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($programs, [StringComparison]::OrdinalIgnoreCase) } |
        Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

if ($Uninstall) {
    Stop-Installed
    Remove-ItemProperty -Path $run -Name $name -ErrorAction SilentlyContinue
    Remove-Item $shortcut -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $programs -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $env:LOCALAPPDATA 'cranked-audio\cranked-audio.sock') -ErrorAction SilentlyContinue
    "Removed. Your settings are still in $env:APPDATA\cranked-audio."
    exit 0
}

if (-not $From) {
    # a release folder has the .exe files next to this script; a source checkout has them under target\
    $candidates = @($PSScriptRoot, "$PSScriptRoot\target\x86_64-pc-windows-msvc\release", "$PSScriptRoot\target\release")
    $From = $candidates | Where-Object { Test-Path (Join-Path $_ 'cranked-audiod.exe') } |
        Sort-Object { (Get-Item (Join-Path $_ 'cranked-audiod.exe')).LastWriteTime } -Descending | Select-Object -First 1
    if (-not $From) { throw 'No cranked-audiod.exe found; build it first (cargo build --release) or pass -From DIR' }
}
foreach ($exe in 'cranked-audiod.exe', 'cranked-audio.exe') {
    if (-not (Test-Path (Join-Path $From $exe))) { throw "$exe is missing in $From" }
}

Stop-Installed
New-Item -ItemType Directory -Force $programs | Out-Null
foreach ($exe in 'cranked-audiod.exe', 'cranked-audio.exe') {
    Copy-Item -Force (Join-Path $From $exe) (Join-Path $programs $exe)
    # a downloaded zip marks its files as from the internet; installing them is the user's go-ahead
    Unblock-File (Join-Path $programs $exe)
}
$service = Join-Path $programs 'cranked-audiod.exe'
$window = Join-Path $programs 'cranked-audio.exe'

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = $window
$link.WorkingDirectory = $programs
$link.Description = 'Per-app volume knobs, on a USB mixer or on screen'
$link.Save()

# start the service (it has the tray icon) at every sign-in
Set-ItemProperty -Path $run -Name $name -Value "`"$service`""
Start-Process $service -WorkingDirectory $programs
"Installed. Cranked Audio is running and starts at sign-in; open it from the Start menu or the tray."
